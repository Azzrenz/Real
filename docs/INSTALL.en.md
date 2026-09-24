# Real — Installation & Getting Started

**Version**: v0.1.0 · Windows x64
**Repository**: https://github.com/Azzrenz/Real

Real is a **local-first AI agent workbench**. Give it a project directory and a one-line request;
it works inside that directory — reading code, editing files, running commands, and leaving
evidence at every step.

---

## 1. Two ways to install — pick one

### Option 1 — Installer (recommended for most people)

**File**: `Real_0.1.0_x64_en-US.msi` (~215 MB)

1. Double-click the `.msi`
2. Click **Next → Install** (it will ask for administrator rights — that's expected)
3. A **Real** shortcut appears on your desktop — double-click to launch

**No Node.js, no Rust, no runtime required.** The backend executable and the WebView2 runtime are
both bundled inside the installer.

- Installs to: `C:\Program Files\Real`
- Best for: people who just want to install and use it

### Option 2 — Portable (USB stick, another PC, no installation)

**File**: `Real-0.1.0-win-x64.zip` (~11 MB)

1. Extract the zip anywhere (a USB stick works)
2. Double-click **`real-client.exe`**

**Your data travels with the folder** — copy the folder to another machine and your sessions and
project memories come along. "Uninstalling" means deleting the folder.

- Requirement: the system must already have the **WebView2 runtime**.
  Windows 11 and recent Windows 10 ship with it; older systems should use Option 1 instead.
- Best for: carrying it around without installing anything

---

## 2. First launch

- **The first launch takes a few seconds** — the backend has to create its database and start local
  services. Every launch after that is fast.
- If the window doesn't appear immediately, **wait ~10 seconds** rather than double-clicking again.

## 3. Do I need an API key?

**No.** Without a configured key, Real automatically uses a **built-in mock model** — every flow
works end to end, the responses just aren't real. It's a fine way to look around first.

**To connect a real model:**

1. Open Real → **Settings** (bottom-left)
2. Paste a **DeepSeek** API key (get one at https://platform.deepseek.com)
3. Save

## 4. Where is my data?

By default:

```
%APPDATA%\real-agent
(i.e. C:\Users\<you>\AppData\Roaming\real-agent)
```

It contains your database (chat history), logs, and project memories.

**To relocate it**: set the data directory in Settings, or set the `REAL_DATA_DIR` environment
variable.

> Portable bonus: when the whole folder is moved, your data moves with it.

## 5. Uninstalling

| | How |
|---|---|
| Option 1 (installer) | Settings → Apps → find Real → Uninstall. The data folder `%APPDATA%\real-agent` is removed separately, if you want it gone |
| Option 2 (portable) | Delete the folder. That's it |

## 6. Notes

- The program **listens on the loopback address only** (`127.0.0.1`) and has **no authentication** —
  it is designed for single-user local use
- **Do not expose it to the public internet**
  (see [SECURITY.md](https://github.com/Azzrenz/Real/blob/main/docs/SECURITY.md))
- It **reads/writes files and executes commands** inside the project directory you give it — that is
  the point of the tool. Point it only at directories you trust
- Risky operations go through a confirmation prompt, but treat that as a way to reduce mistakes,
  **not as a security boundary**

---

MIT License · © 2026 RealBody
