# /// script
# requires-python = ">=3.10,<3.13"
# dependencies = ["demucs>=4.0.1", "torch", "torchaudio", "soundfile", "numpy<2"]
# ///
"""Split tracks into stems, once per track, the same way every time.

    uv run --script scripts/stems.py [--model htdemucs_ft] [--device auto] [--force] FILE...

Models:
  htdemucs_ft (default)  Demucs, fine-tuned: drums, bass, other, vocals. Lands in <name>.stems/.
  htdemucs, htdemucs_6s  Other Demucs models; htdemucs_6s adds guitar and piano.
  BS-Roformer-SW.ckpt    Or any other audio-separator model file (a name with an extension):
                         run through audio-separator, which is fetched on first use.
                         BS-Roformer-SW makes six: bass, drums, guitar, piano, vocals, other.

Each track's stems go beside it: the default model's in `<name>.stems/`, any other's in
`<name>.stems-<model>/` (the model without its extension), so one track can hold several
splits and adeck switches between them. A folder holds one FLAC per stem at the track's own
rate and length, and `stems.json`: the model, the stems it made (`stems`), and each stem's
loudness. A track whose `stems.json` matches its size, date and the model is skipped.

Splits are deterministic: before each track, Python's, numpy's and torch's generators are
seeded from a hash of the file's bytes (Demucs's time shift is a random.randint), and CUDA's
cudnn runs in deterministic mode. Same file, same model, same machine: the same stems.

One JSON line per track goes to stdout as it finishes, for adeck's job panel. The model's
weights are downloaded on first use and never bundled: Demucs's code is MIT but its weights'
licence is unclear upstream, and audio-separator's models each carry their own.
"""
import argparse, hashlib, json, os, random, shutil, subprocess, sys, tempfile, time

import numpy as np

DEFAULT = "htdemucs_ft"
# adeck's lane order; stems a model names beyond these keep the model's order after them.
ORDER = ["drums", "bass", "guitar", "piano", "vocals", "other"]
VERSION = 1
LEVELS = 2
REEXEC = "ADECK_STEMS_SEPARATOR"


def say(**kw):
    print(json.dumps(kw), flush=True)


def ordered(names):
    return sorted(names, key=lambda n: ORDER.index(n) if n in ORDER else len(ORDER))


def is_separator(model):
    """audio-separator models are named by file (BS-Roformer-SW.ckpt, htdemucs_6s.yaml)."""
    return os.path.splitext(model)[1] in (".ckpt", ".yaml", ".onnx", ".pth", ".th")


def suffix(model):
    return "" if model == DEFAULT else os.path.splitext(model)[0]


def out_dir(path, model):
    base, _ = os.path.splitext(path)
    s = suffix(model)
    return base + ".stems" + (f"-{s}" if s else "")


def fresh(path, model):
    d = out_dir(path, model)
    try:
        meta = json.load(open(os.path.join(d, "stems.json")))
    except (OSError, ValueError):
        return False
    st = os.stat(path)
    ok = meta.get("version") == VERSION and meta.get("model") == model
    ok = ok and meta.get("source_size") == st.st_size and abs(meta.get("source_mtime", 0) - st.st_mtime) < 1
    return ok and all(os.path.isfile(os.path.join(d, f"{s}.flac")) for s in meta.get("stems") or ["?"])


def seed_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return int(h.hexdigest()[:8], 16)


def seed_all(s):
    import torch
    random.seed(s)
    np.random.seed(s)
    torch.manual_seed(s)
    if torch.cuda.is_available():
        torch.cuda.manual_seed_all(s)


def probe_rate(path):
    out = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "a:0", "-show_entries", "stream=sample_rate", "-of", "csv=p=0", path], capture_output=True, text=True)
    try:
        return int(out.stdout.strip().split("\n")[0])
    except ValueError:
        raise RuntimeError(f"ffprobe can't read {path}: {out.stderr.strip()[-200:]}")


def decode(path, rate):
    """Stereo float32 at the file's own rate, encoder padding trimmed (as adeck decodes it)."""
    out = subprocess.run(["ffmpeg", "-v", "error", "-i", path, "-vn", "-f", "f32le", "-ac", "2", "-ar", str(rate), "-"], capture_output=True)
    if out.returncode != 0:
        raise RuntimeError(f"ffmpeg can't decode {path}: {out.stderr.decode()[-200:]}")
    return np.frombuffer(out.stdout, dtype=np.float32).reshape(-1, 2).T.copy()


def levels(parts, mix, rate):
    """Each stem's loudness (BS.1770 LUFS, gated, as meters show it) and its share of the
    track's K-weighted energy over the whole length, ungated, so a part that's silent for most
    of the track counts as small: "the bass is 40% of what you hear" reads off the tile.
    `parts` is {stem: [2, n] tensor}."""
    import torchaudio.functional as AF
    def lufs(x):
        v = float(AF.loudness(x, rate))
        return v if v == v and v > -70 else -70.0
    def energy(x):
        # BS.1770's K-weighting (the same two filters torchaudio's loudness uses), then mean power.
        k = AF.highpass_biquad(AF.treble_biquad(x, rate, 4.0, 1500.0, 1 / 2 ** 0.5), rate, 38.0, 0.5)
        return float((k ** 2).mean(-1).sum())
    es = {s: energy(p) for s, p in parts.items()}
    tot = sum(es.values()) or 1.0
    return {"v": LEVELS, "mix_lufs": round(lufs(mix), 2), "stems": {s: {"lufs": round(lufs(p), 2), "share": round(es[s] / tot, 4)} for s, p in parts.items()}}


def residual_db(parts, mix):
    """How far the stems' sum is from the mix, in dB under the mix: lower is a cleaner split."""
    import torch
    err = sum(parts.values()) - mix
    return round(10 * float(torch.log10((err ** 2).mean() / (mix ** 2).mean().clamp_min(1e-12))), 1)


def fill_levels(path, model):
    """Adds loudness to stems split before it was measured."""
    d = out_dir(path, model)
    meta = json.load(open(os.path.join(d, "stems.json")))
    if meta.get("levels", {}).get("v") == LEVELS:
        return
    import torch, soundfile as sf
    names = meta.get("stems") or ["drums", "bass", "other", "vocals"]
    parts = {s: torch.from_numpy(sf.read(os.path.join(d, f"{s}.flac"), dtype="float32", always_2d=True)[0].T.copy()) for s in names}
    n = min(p.shape[-1] for p in parts.values())
    mix = torch.from_numpy(decode(path, meta["rate"]))[..., :n]
    meta["levels"] = levels({s: p[..., :n] for s, p in parts.items()}, mix, meta["rate"])
    json.dump(meta, open(os.path.join(d, "stems.json"), "w"), indent=1)


class Demucs:
    def __init__(self, model, dev):
        from demucs.pretrained import get_model
        self.model = get_model(model)
        self.model.eval()
        self.dev = dev
        self.rate = self.model.samplerate

    def split(self, x, rate):
        """{stem: [2, n]} at `rate` from a [2, n] mix."""
        import torch, torchaudio.functional as AF
        from demucs.apply import apply_model
        y = AF.resample(x, rate, self.rate) if rate != self.rate else x
        ref = y.mean(0)
        mu, sd = ref.mean(), ref.std().clamp_min(1e-8)
        with torch.no_grad():
            # shifts=1: one random time offset (seeded), the quality/speed point Demucs suggests.
            parts = apply_model(self.model, ((y - mu) / sd)[None], device=self.dev, shifts=1, split=True, overlap=0.25, progress=False)[0]
        parts = parts * sd + mu / len(self.model.sources)
        if rate != self.rate:
            parts = AF.resample(parts, self.rate, rate)
        return dict(zip(self.model.sources, parts))


class Separator:
    """audio-separator (python-audio-separator): RoFormer, MDX and the rest, by model file."""

    def __init__(self, model, dev):
        from audio_separator.separator import Separator as S
        self.tmp = tempfile.mkdtemp(prefix="adeck-sep-")
        models = os.environ.get("ADECK_SEPARATOR_MODELS") or os.path.join(os.path.expanduser("~"), ".cache", "audio-separator-models")
        self.sep = S(output_dir=self.tmp, model_file_dir=models, output_format="WAV", log_level=30)
        self.sep.load_model(model_filename=model)

    def split(self, x, rate):
        import torch, soundfile as sf, torchaudio.functional as AF
        # Our own decode goes in, so the stems line up with the mix to the sample.
        out = os.path.join(self.tmp, "out")
        shutil.rmtree(out, ignore_errors=True)
        os.makedirs(out)
        self.sep.output_dir = out
        if getattr(self.sep, "model_instance", None) is not None:
            self.sep.model_instance.output_dir = out
        src = os.path.join(self.tmp, "mix.wav")
        sf.write(src, x.T.numpy(), rate, subtype="FLOAT")
        parts = {}
        for f in self.sep.separate(src):
            f = f if os.path.isabs(f) else os.path.join(out, f)
            name = os.path.basename(f).split("_(", 1)[1].split(")", 1)[0].lower()
            y, ysr = sf.read(f, dtype="float32", always_2d=True)
            y = torch.from_numpy(y.T.copy())
            parts[name] = AF.resample(y, ysr, rate) if ysr != rate else y
        return parts


def with_separator():
    """audio-separator isn't in the header (it's heavy and few need it): run this script again
    with it added, once. (audioread: audio-separator imports it without depending on it.)"""
    try:
        import audio_separator  # noqa: F401
        return
    except ImportError:
        pass
    uv = os.environ.get("UV") or shutil.which("uv")
    if os.environ.get(REEXEC) or not uv:
        sys.exit("audio-separator models need the audio-separator package: run with uv, or pip install 'audio-separator[cpu]'")
    os.environ[REEXEC] = "1"
    args = [uv, "run", "--quiet", "--python", "3.12", "--with", "audio-separator[cpu]", "--with", "audioread", "--script", os.path.abspath(__file__), *sys.argv[1:]]
    os.execv(uv, args)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("files", nargs="+")
    ap.add_argument("--model", default=DEFAULT)
    ap.add_argument("--device", default="auto")
    ap.add_argument("--force", action="store_true")
    a = ap.parse_args()

    todo = [f for f in a.files if a.force or not fresh(f, a.model)]
    for f in a.files:
        if f not in todo:
            try:
                fill_levels(f, a.model)
            except Exception as e:
                say(file=f, status="cached", dir=out_dir(f, a.model), warning=f"no levels: {str(e)[-200:]}")
                continue
            say(file=f, status="cached", dir=out_dir(f, a.model))
    if not todo:
        return

    sep = is_separator(a.model)
    if sep:
        with_separator()
    os.environ.setdefault("PYTORCH_ENABLE_MPS_FALLBACK", "1")
    import torch, soundfile as sf

    dev = a.device
    if dev == "auto":
        dev = "mps" if torch.backends.mps.is_available() else "cuda" if torch.cuda.is_available() else "cpu"
    # Same input, same stems: no autotuned (and run-to-run different) kernels on CUDA.
    torch.backends.cudnn.deterministic, torch.backends.cudnn.benchmark = True, False
    t = time.time()
    splitter = Separator(a.model, dev) if sep else Demucs(a.model, dev)
    say(status="model", model=a.model, device=dev, secs=round(time.time() - t, 2))

    for i, f in enumerate(todo):
        t = time.time()
        try:
            rate = probe_rate(f)
            x = torch.from_numpy(decode(f, rate))
            n = x.shape[-1]
            seed = seed_of(f)
            seed_all(seed)
            got = splitter.split(x, rate)
            names = ordered(got)
            parts = {}
            for s in names:
                p = got[s][..., :n]
                parts[s] = torch.nn.functional.pad(p, (0, n - p.shape[-1])) if p.shape[-1] < n else p
            final = out_dir(f, a.model)
            tmp = final + ".part"
            shutil.rmtree(tmp, ignore_errors=True)
            os.makedirs(tmp)
            for s, p in parts.items():
                sf.write(os.path.join(tmp, f"{s}.flac"), p.T.numpy(), rate, subtype="PCM_16")
            st = os.stat(f)
            meta = {"version": VERSION, "model": a.model, "source": os.path.basename(f), "source_size": st.st_size, "source_mtime": st.st_mtime, "rate": rate, "frames": n, "stems": names}
            meta["levels"] = levels(parts, x, rate)
            meta["residual_db"] = residual_db(parts, x)
            meta["deterministic"] = "seeded from sha256(file)"
            meta["seed"] = seed
            json.dump(meta, open(os.path.join(tmp, "stems.json"), "w"), indent=1)
            shutil.rmtree(final, ignore_errors=True)
            os.rename(tmp, final)
            say(file=f, status="done", dir=final, stems=names, i=i + 1, n=len(todo), secs=round(time.time() - t, 2))
        except Exception as e:  # one bad file mustn't stop the batch
            say(file=f, status="failed", error=str(e)[-300:], i=i + 1, n=len(todo))


if __name__ == "__main__":
    main()
