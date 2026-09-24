# Real

> A local-first AI agent workbench that actually gets work done.

Point it at a project directory, describe what you need in one sentence, and it works **inside that
directory** — reading code, editing files, running commands, leaving evidence at every step.

Unlike a chat app, it runs a **convergence loop**: the model takes one step, executes it, sees the
result, and keeps going until it has something to deliver. Every state change is persisted, every
round is traceable, every edit is diffable.

**Backend** Rust (Axum + SQLx + SQLite) · **Desktop shell** Tauri 2 + React 18
**Bilingual UI** (English / 中文) · MIT License · Windows fully verified

> **The [Chinese README](../README.md) is the authoritative document.** It contains the full mechanism
> descriptions, architecture notes and design rationale. This English file is a front-page overview
> so that non-Chinese readers can understand what Real is and how to run it.

---

## What it is

- **Local-first** — everything runs on your machine. No cloud sync, no multi-tenancy, no accounts.
- **Four-tool surface** — `read` / `write` / `modify` / `run`, deliberately minimal.
- **Evidence over narration** — the backend scans the workspace itself (by file mtime) to know what
  actually changed. The model cannot simply claim it did something.
- **Safe by default** — dangerous operations pass through a confirmation gate; the server binds to
  `127.0.0.1` only and validates the request `Origin`.
- **Auditable** — all state lives in SQLite: sessions, events, per-round usage and memories.

![Real main window](../screenshots/01-welcome.png)

A task in progress — user request, model narration, tool calls, and the work rules injected for
that task:

![Real running](../screenshots/02-running.png)

## Why I built it

I'm a music producer, not a programmer. I wanted an AI assistant inside my arranging workflow —
"add a drum part to this guitar", "write a bassline that follows this melody". Making that reliable
required something I could control: my own agent platform, running locally, leaving a trail, never
flying blind.

So Real exists as **the tool I built before the product**. Its tens of thousands of lines of code
were written without me typing a single line myself.

## What's inside

| | |
|---|---|
| **Convergence loop** | decide → execute → feedback → deliver |
| **Stall protection** | escalating thresholds at 4 / 8 / 12 zero-progress rounds, plus a 30-round diagnostic fallback and a 20-round floor |
| **Prompts treated as code** | each prompt file owns one responsibility, one injection point, one timing |
| **Knowledge on demand** | work rules are injected only when their trigger keywords match the task |
| **Cost governance** | hot/cold cache gate, read cache, long-output spill archiving, session retirement |
| **MCP · scheduling · long-term memory** | built in |

## Install

Full walkthrough: **[INSTALL.en.md](INSTALL.en.md)**

**Option 1 — Installer (recommended).** Download `Real_0.1.0_x64_en-US.msi` from
[Releases](https://github.com/Azzrenz/Real/releases/latest) and double-click it.
No Node.js, no Rust, no runtime needed — the WebView2 runtime is bundled.

**Option 2 — Portable.** Download the `.zip`, extract it anywhere (a USB stick is fine) and run
`real-client.exe`. Your data travels with the folder; uninstalling means deleting it.

> First launch takes a few seconds while the backend creates its database and starts local services.
> **No API key is required** — without one, Real falls back to a built-in mock model so you can walk
> through every flow before wiring up a real provider.

### Bring your own model

**Two providers ship built in**: DeepSeek and Zhipu GLM — the model chip at the bottom of the app
switches between them in one click (the backend swaps both endpoint and key together).

To add a **third** provider (Kimi, Qwen, a self-hosted proxy — anything), you **don't touch code**:

1. Drop a `<vendor-id>.json` into `server/assets/providers/`
2. Restart Real (the startup log prints a line when it picks the file up)
3. Enter that provider's API key **once** in Settings

That directory is the **provider registry** — one JSON file per vendor. OpenAI-compatible vendors can
copy the template as-is.

**Full field reference + template + the two easy-to-miss pitfalls** (written in Chinese):

→ [`../server/assets/providers/接入新公司-读我.md`](../server/assets/providers/接入新公司-读我.md)

> The pitfall worth repeating here: **`id` and `label` are different things, and both are required.**
> `id` is the `model` parameter sent upstream — it must match the vendor's docs exactly or you get a
> 400 immediately. `label` is the human-readable version shown in the UI; skip it and you'll only see
> the API name, with no way to tell which version you're actually paying for.

## Build from source

Requires Rust 1.75+ and Node.js 18+.

```bash
# backend
cd server
cargo build
./target/debug/real-server        # listens on 127.0.0.1:8943

# desktop shell (second terminal)
cd client
npm install
npm run tauri dev                 # Vite dev server on 8618
```

On Windows you can simply double-click **`build-run.bat`** — it checks the toolchain, creates
`.env` from the template, installs dependencies, and starts both processes.

## Architecture

```
Real/
├── server/                    Rust backend
│   ├── src/
│   │   ├── agent/             convergence loop · context assembly · history · memory
│   │   ├── tools/             the four tools + contracts + validation
│   │   ├── mcp/               MCP client · tool registry · spill archiving
│   │   ├── model/             LLM client · model catalog
│   │   ├── routes/            HTTP API
│   │   ├── db/                pool · repositories
│   │   ├── config/            config · runtime settings
│   │   ├── sse/               event stream
│   │   ├── scheduler/         scheduled triggers
│   │   └── path/              paths · data root
│   ├── prompts/               prompt assets (compile-time embedded)
│   ├── specs/                 work rules, injected on demand
│   └── migrations/            SQLite migrations
└── client/                    Tauri 2 shell + React 18
    └── src/                   features · services · shared · styles
```

## Contributing

Please read [CONTRIBUTING.md](CONTRIBUTING.md) first. The single most important rule:

> Before touching anything, answer three questions: which department does this belong to?
> Can an existing department host it? Is this an architecture gap or an implementation bug?

- **Bugs, feature requests, questions** → [open an issue](https://github.com/Azzrenz/Real/issues)
- **Security issues** → follow [SECURITY.md](SECURITY.md) and **do not** open a public issue

## Status

Early (`0.x`). Windows is fully verified; macOS and Linux compile but are **untested** —
we state that plainly rather than claiming a support matrix we haven't run.
Part of the settings panel is not fully wired up yet; see the section
「还没做完的部分」in the [Chinese README](../README.md).

## License

[MIT](../LICENSE) © 2026 RealBody
