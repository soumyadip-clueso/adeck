# /// script
# requires-python = ">=3.10,<3.13"
# dependencies = ["demucs>=4.0.1", "torch", "torchaudio", "soundfile", "numpy<2"]
# ///
"""Split tracks into stems (drums, bass, other, vocals) with Demucs, once per track.

    uv run --script scripts/stems.py [--model htdemucs_ft] [--device auto] [--force] FILE...

Each track's stems go beside it, in `<name>.stems/`: one FLAC per stem at the track's own rate
and length, and `stems.json` saying what made them. A track whose `stems.json` matches its size,
date and the model is skipped. One JSON line per track goes to stdout as it finishes, for
adeck's job panel. The model's weights are downloaded on first use (Demucs's code is MIT; the
weights' licence is unclear upstream, so they're never bundled).
"""
import argparse, json, os, shutil, subprocess, sys, time

import numpy as np

STEMS = ["drums", "bass", "other", "vocals"]
VERSION = 1


def say(**kw):
    print(json.dumps(kw), flush=True)


def out_dir(path):
    base, _ = os.path.splitext(path)
    return base + ".stems"


def fresh(path, model):
    try:
        meta = json.load(open(os.path.join(out_dir(path), "stems.json")))
    except (OSError, ValueError):
        return False
    st = os.stat(path)
    ok = meta.get("version") == VERSION and meta.get("model") == model
    ok = ok and meta.get("source_size") == st.st_size and abs(meta.get("source_mtime", 0) - st.st_mtime) < 1
    return ok and all(os.path.isfile(os.path.join(out_dir(path), f"{s}.flac")) for s in STEMS)


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


LEVELS = 2


def levels(parts, mix, rate):
    """Each stem's loudness (BS.1770 LUFS, gated, as meters show it) and its share of the
    track's K-weighted energy over the whole length, ungated, so a part that's silent for most
    of the track counts as small: "the bass is 40% of what you hear" reads off the tile."""
    import torch, torchaudio.functional as AF
    def lufs(x):
        v = float(AF.loudness(x, rate))
        return v if v == v and v > -70 else -70.0
    def energy(x):
        # BS.1770's K-weighting (the same two filters torchaudio's loudness uses), then mean power.
        k = AF.highpass_biquad(AF.treble_biquad(x, rate, 4.0, 1500.0, 1 / 2 ** 0.5), rate, 38.0, 0.5)
        return float((k ** 2).mean(-1).sum())
    ls = {s: lufs(p) for s, p in zip(STEMS, parts)}
    es = {s: energy(p) for s, p in zip(STEMS, parts)}
    tot = sum(es.values()) or 1.0
    return {"v": LEVELS, "mix_lufs": round(lufs(mix), 2), "stems": {s: {"lufs": round(ls[s], 2), "share": round(es[s] / tot, 4)} for s in STEMS}}


def fill_levels(path):
    """Adds loudness to stems split before it was measured."""
    d = out_dir(path)
    meta = json.load(open(os.path.join(d, "stems.json")))
    if meta.get("levels", {}).get("v") == LEVELS:
        return
    import torch, soundfile as sf
    parts = torch.stack([torch.from_numpy(sf.read(os.path.join(d, f"{s}.flac"), dtype="float32", always_2d=True)[0].T.copy()) for s in STEMS])
    mix = torch.from_numpy(decode(path, meta["rate"]))[..., : parts.shape[-1]]
    meta["levels"] = levels(parts, mix, meta["rate"])
    json.dump(meta, open(os.path.join(d, "stems.json"), "w"), indent=1)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("files", nargs="+")
    ap.add_argument("--model", default="htdemucs_ft")
    ap.add_argument("--device", default="auto")
    ap.add_argument("--force", action="store_true")
    a = ap.parse_args()

    todo = [f for f in a.files if a.force or not fresh(f, a.model)]
    for f in a.files:
        if f not in todo:
            try:
                fill_levels(f)
            except Exception as e:
                say(file=f, status="cached", dir=out_dir(f), warning=f"no levels: {str(e)[-200:]}")
                continue
            say(file=f, status="cached", dir=out_dir(f))
    if not todo:
        return

    os.environ.setdefault("PYTORCH_ENABLE_MPS_FALLBACK", "1")
    import torch, torchaudio.functional as AF, soundfile as sf
    from demucs.pretrained import get_model
    from demucs.apply import apply_model

    dev = a.device
    if dev == "auto":
        dev = "mps" if torch.backends.mps.is_available() else "cuda" if torch.cuda.is_available() else "cpu"
    t = time.time()
    model = get_model(a.model)
    model.eval()
    if list(model.sources) != STEMS:
        sys.exit(f"{a.model} makes {model.sources}, not {STEMS}")
    say(status="model", model=a.model, device=dev, secs=round(time.time() - t, 2))

    for i, f in enumerate(todo):
        t = time.time()
        try:
            rate = probe_rate(f)
            x = torch.from_numpy(decode(f, rate))
            n = x.shape[-1]
            y = AF.resample(x, rate, model.samplerate) if rate != model.samplerate else x
            ref = y.mean(0)
            mu, sd = ref.mean(), ref.std().clamp_min(1e-8)
            with torch.no_grad():
                parts = apply_model(model, ((y - mu) / sd)[None], device=dev, shifts=1, split=True, overlap=0.25, progress=False)[0]
            parts = parts * sd + mu / len(STEMS)
            if rate != model.samplerate:
                parts = AF.resample(parts, model.samplerate, rate)
            parts = parts[..., :n]
            if parts.shape[-1] < n:
                parts = torch.nn.functional.pad(parts, (0, n - parts.shape[-1]))
            final = out_dir(f)
            tmp = final + ".part"
            shutil.rmtree(tmp, ignore_errors=True)
            os.makedirs(tmp)
            for s, p in zip(STEMS, parts):
                sf.write(os.path.join(tmp, f"{s}.flac"), p.T.numpy(), rate, subtype="PCM_16")
            st = os.stat(f)
            meta = {"version": VERSION, "model": a.model, "source": os.path.basename(f), "source_size": st.st_size, "source_mtime": st.st_mtime, "rate": rate, "frames": n, "stems": STEMS}
            meta["levels"] = levels(parts, x, rate)
            json.dump(meta, open(os.path.join(tmp, "stems.json"), "w"), indent=1)
            shutil.rmtree(final, ignore_errors=True)
            os.rename(tmp, final)
            say(file=f, status="done", dir=final, i=i + 1, n=len(todo), secs=round(time.time() - t, 2))
        except Exception as e:  # one bad file mustn't stop the batch
            say(file=f, status="failed", error=str(e)[-300:], i=i + 1, n=len(todo))


if __name__ == "__main__":
    main()
