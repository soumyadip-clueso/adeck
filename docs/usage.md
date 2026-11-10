# Using adeck

- [The window](#the-window)
- [Views](#views)
- [Listening](#listening)
- [Telling the agent](#telling-the-agent)
- [Verdicts](#verdicts)
- [Screenshots and sharing](#screenshots-and-sharing)
- [Keyboard](#keyboard)

## The window

Every tab has the same frame:

- **The page** in the middle: the view itself.
- **The player** along the bottom: play, the clock, an overview to scrub, zoom, volume, then what the view decides with (score keys, the take you hear and Pick) and Send to agent.
- **The pane** beside the page: everything about the sound that isn't the sound itself, in sections (metadata, the diff, earshot, the take you hear, your note). Dock it left or right with the buttons in its head or by dragging its grip across, resize it from its inner edge, and hide it to a thin tab with `\`.
- **The footer** shows running jobs (downloads, analysis). A finished job leaves after a few seconds; a failed one stays until you dismiss it. Click the summary for Activity, which keeps every job and its log until Clear all.

Tabs come back when you reopen adeck. **+** after the tabs (⌘T) opens a file (⌘O), a folder, a new empty compare, or a run. Drop audio files on a single file or a compare to add them.

## Views

| View | What it's for |
|---|---|
| **One file** | The waveform with its beat grid, a spectrogram, loudness, peak, tempo and key, every metadata field found for it, and earshot's description and measurements. Score it 0 to 3. |
| **Compare** (2 to 9 files) | Every waveform on one time axis with one playhead. All play at once and you hear one; switching is instant and keeps the position. Loudness matching (on by default) brings each to the same LUFS so the louder take doesn't win by being louder. Blind hides names as X, Y, Z until you pick. The pane's DIFF shows only the metadata that differs. |
| **Render** | The video in sync with its audio, earshot's scorecard, and every cut marked by whether it got a hit, some sound, or silence. Several renders switch like a compare. |
| **Run** | Every item of a run as a sortable, filterable table. Show what you haven't scored, open one, or compare a selection. |
| **Board** | One card per group (a category, say), sorted by what the agent says matters, each a row of tiles: the picks, the raw "before" and the references, with the agent's numbers and a verdict chip. Click a tile to play it. The group in focus opens in the pane: the take zoomed to its sound, what made it, keep / drop / move-to, and the rest of the takes, ranked. The agent adds or replaces groups while it's open. |
| **Pads** | For one-shot effects. Each file is a big button: click it or press its key and it plays from the top, cutting off the last one. Score and keep or drop right on the pad. |
| **Diff** | Run A against run B, one row per group and one column per metric, each change coloured by which way is better. A row naming a file from each side opens them as a compare. |
| **Shared** | Every page you've shared, with Open, Copy link and Take down. |

## Listening

- **Scrub**: press or drag on any waveform or spectrogram, or on the player's overview.
- **Select a stretch**: shift-drag across a strip. Space then plays just that stretch and stops at its end; L loops it; shift-click or Esc clears it.
- **Loop**: L loops the selection, or drag along the time ruler above the strips. Click the ruler or press Esc to clear it.
- **Speed**: the 1× key in the player (or X) plays at 2× with the pitch kept, for skimming long takes.
- **Download**: the arrow at a board tile's top right saves a copy of the file (Downloads to start with). It hides while a group is blind.
- **Zoom**: − and + (or = and −) zoom around the playhead, F fits the file or the loop, [ and ] pan. A zoomed view follows the playhead while it plays.
- **Volume**: drag across the bars in the player.

## Telling the agent

- **Send to agent**: double-click a strip, a tile or a compare member, or press S. The agent gets what you're on (the tab, group, take, the loop or selection, the playhead and your note) through `get_selection`, and a line is copied for you to paste into the chat.
- **Your note**: N opens it in the pane. The agent's own note for the tab ("what to listen for") shows on the page; fold or dismiss it.

## Verdicts

Every score, pick, note and keep / drop / move is one JSON line in `~/.config/adeck/feedback.jsonl`. When all of a tab's files sit in one local run, the line also goes into that run's own `feedback.jsonl`. A blind pick records which label was which. The agent reads them with `get_feedback`, or waits for the next one with `wait_for_feedback`.

## Screenshots and sharing

- **Screenshot** (⌘⇧S, the camera in the footer, or `screenshot_tab` with `pretty: true`): the tab at 2x on a gradient with rounded corners and a shadow, saved to `~/Pictures/adeck` and copied to the clipboard.
- **Share** (⌘⇧E, the Share button at the top right, the share icon in the footer, or `share_tab`): the tab becomes a web page with gapless A/B players, waveforms, numbers and details, and the link is copied. Files from your reference pack are never uploaded; they show as local-only.
- **Take down** (the Shared tab, or `unshare`): deletes the page and its audio. A page taken down elsewhere shows as down, and its uploaded audio is cleared.

Sharing calls a publisher program you name in the config, so any host works:

```toml
[share]
command = "~/.config/adeck/my-publisher"   # publish <dir> <slug> <title> → {"url": …}; remove <slug>; list → [{slug, url}]
prefix = "adeck-"
audio = "bundle"                           # audio inside the page, or "s3" with [share.s3] below
# [share.s3]
# bucket = "…"; prefix = "shares"; profile = "…"; region = "…"; public_url = "https://…"

# Files that are already public (say, downloaded from a bucket a CDN serves) aren't copied:
# the page plays them from <url>/<path under dir>. Each is checked first (HEAD, same size);
# anything that doesn't match is copied as usual. Repeat the block per folder.
[[share.links]]
dir = "~/music/run-12"
url = "https://cdn.example.com/runs/run-12/"
```

A big board with linked files shares fine; bundled audio is limited by what your host accepts.

## Keyboard

Press `?` in the app for this list.

**Everywhere**

| Key | Does |
|---|---|
| ⌘K | Search or run a command (tabs, groups, sounds, runs, shared pages) |
| ⌘F or / | Search this tab |
| ⌘T / ⌘O / ⌘W | New tab menu / open a file / close the tab |
| ⌘⇧E / ⌘⇧S | Share the tab / screenshot it |
| `\` | Hide or show the pane |
| ⌘1–5 | Open a pane section |
| ⌘[ / ⌘] | Dock the pane left / right |
| N | Your note for the agent |
| ? | Every key |

**Listening**

| Key | Does |
|---|---|
| Space | Play or pause (just the selection, if there is one) |
| ← → | Step a second (with ⇧, five) |
| = − | Zoom in or out |
| F | Fit the file or the loop |
| [ ] | Pan |
| ⇧ drag | Select a stretch |
| L | Loop the selection |
| X | Play at 1× or 2× |
| Esc | Clear the loop and selection |
| S | Send to agent |

**Compare and one file**

| Key | Does |
|---|---|
| 1–9 | Switch to a take |
| ↵ | Pick the one you hear |
| B | Blind on or off |
| M | Loudness match on or off |
| 0–3 | Score (one file) |

**Board**

| Key | Does |
|---|---|
| ↑ ↓ | Previous or next group |
| ← → | Previous or next tile |
| 1–9 | Play a tile |
| R | Replay from the start of the sound |
| L | Loop the selection, or what the strip shows |
| Z | Zoom to the sound or the whole file |
| K / D | Keep / drop |
| ↵ | Pick |
| Esc | Clear the loop, or hide the pane |

**Pads**

| Key | Does |
|---|---|
| 1–0, Q–P | Play a pad from the top |
| Space | Replay the last pad |
| Z X C V | Score it 0 to 3 |
