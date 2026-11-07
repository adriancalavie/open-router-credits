# open-router-credits

[![CI](https://github.com/adriancalavie/open-router-credits/actions/workflows/rust.yml/badge.svg?branch=main&label=CI)](https://github.com/adriancalavie/open-router-credits/actions/workflows/rust.yml)

Shows your OpenRouter credit balance in the terminal.

```
╭ open-router-credits ─────────────────────────────────────────────────────╮
│                                                                          │
│METRIC                                                      USD     SHARE │
│Purchased                                               $100.50    100.0% │
│Used                                                     $25.75     25.6% │
│Remaining                                                $74.75     74.4% │
│                                                                          │
│[█████████████████████████████████████████████░░░░░░░░░░░░░░░] 74.4% left │
│                                                                          │
│ q quit    r refresh    ? help               updated just now · auto 60s  │
│                                                                          │
╰──────────────────────────────────────────────────────────────────────────╯
```

The panel shows the balance of your OpenRouter account and refreshes it every 60 seconds.

## Why use it

- The balance is one glance away. No tab, no login, no dashboard to open.
- It counts every API key, not just the one in your config.
- If a refresh fails, the last value stays on screen with its age.
- `--once` prints the same values as plain text for scripts.

If you want spend per model or per key, use the OpenRouter dashboard.  
This tool answers two questions: "what is left?" and "how much went out lately?"

## Requirements

- Rust 1.88 or newer.
- A management key. The credits endpoint rejects inference keys with `403`. Create one at
  <https://openrouter.ai/settings/management-keys>.

## Install

```sh
cargo install --path .
```

This command puts the binary in `~/.cargo/bin`. That directory is on your `PATH` if you use
rustup.

## First run

```sh
open-router-credits
```

The wizard asks for the management key and hides the input. It tests the key before it saves
it. Then it writes `~/.config/open-router-credits/config.toml` (mode `0600`: only your user can
read or write it).

## Usage

```sh
open-router-credits                   # open the panel
open-router-credits --range week      # open it on a time window
open-router-credits --setup           # change the key
open-router-credits --once            # print the values and exit
open-router-credits --once --range 30d
open-router-credits --config <PATH>   # use another config file
```

Press `q` to quit, `r` to refresh, `0`-`7` to switch the window, and `?` for help. It needs
60 columns and 12 rows.

## Windows

The default view is lifetime: purchased, used, remaining. A window replaces the "Used" row
with what you spent inside it, and swaps the share column for a rate:

```
METRIC                                                      USD       RATE
Used · this week                                          $5.68    $0.97/d
Remaining                                                 $3.50      3.6 d
```

A rate is used because a percentage of lifetime purchases stops meaning anything as the total
grows: $5 out of $10 000 spent is 0.05% and tells you nothing. The rate says "about a dollar a
day, and the balance lasts three and a half days at that pace". Both numbers come from the
window you picked, so a quiet year average is not the same as a busy week — the title says
which one you are looking at.

| key | window | starts at |
| --- | --- | --- |
| `0` | all time | nothing, lifetime totals |
| `1` | this session | when the panel started, no query |
| `2` | this week | Monday 00:00 local |
| `3` | last 7 days | rolling |
| `4` | this month | the 1st, local |
| `5` | last 30 days | rolling |
| `6` | this quarter | the quarter's first day, local |
| `7` | this year | 1 January, local |

In the lifetime view `SHARE` is a slice of everything you ever purchased. In a window that
column becomes `RATE`: dollars per day for the spend row, days of balance left for the
remaining row.

The bar changes with the view too. Lifetime, it reads "how much of everything I bought is
left". In a window it reads "how much of the money that moved inside this window is still
there": remaining / (spent + remaining). Spent $5.68 since Monday with $3.50 left in the
account, and $9.18 moved through the window, so 38.1% of it is left.

Each window is one `POST /api/v1/analytics/query` with an exact start and end instant, so a
window begins at your local midnight, not at a UTC day boundary. Windows refresh every five
minutes; the balance refreshes every `refresh_secs`. Nothing is stored on disk.

Two limits come from the API, not from this tool:

- There is no endpoint for top-ups or for a past balance. A window reports what you spent,
  and the balance is always the current one.
- The server keeps 367 days of daily data, which covers "this year" but not earlier.

## Configuration

```toml
api_key = "sk-or-v1-..."   # created by the wizard
refresh_secs = 60          # 5..3600
# api_base = "https://openrouter.ai/api/v1"
```

Set `OPENROUTER_API_BASE` to use another API address, for example a proxy. Set `NO_COLOR=1`
to remove color. The layout uses bold, faint and reverse video, so it follows the theme of
your terminal.

## Security

The management key lives in a `0600` file (Windows uses the folder permissions). It goes only to
`openrouter.ai` over HTTPS, and is never printed or passed as an argument. Plain `http://` is
refused, except for localhost.

A management key can create and delete your API keys. Treat the file like a password: set an
expiration when you create the key. If you set `api_base` or `OPENROUTER_API_BASE`, the key goes
there, so trust that host.

## Disclaimer

This is a personal tool. It changes when it needs to, not on a schedule. There is no support
promise. Bug reports and patches are welcome.

## Development

```sh
cargo run
cargo test
# The tests use a mocked API.
```

## License

MIT. See [LICENSE](LICENSE).
