# open-router-credits

[![CI](https://github.com/adriancalavie/open-router-credits/actions/workflows/rust.yml/badge.svg?branch=main&label=CI)](https://github.com/adriancalavie/open-router-credits/actions/workflows/rust.yml)
[![rust](https://img.shields.io/badge/rust-1.88%2B-orange.svg?logo=rust)](Cargo.toml)
[![lines](https://img.shields.io/badge/lines-3.1k-blue.svg)](src)

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
│ q quit    0-7 window    r refresh    ? help   updated 4s ago · auto 60s  │
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
16 columns and 5 rows (4 in window mode). It gives up information only when it runs out of room,
one thing at a time: the footer, the bar, the blank line, the header, then within a row the
SHARE/RATE column (around 31 columns), the dollar sign (around 20, and only while the header
still says USD), then label length. Labels shorten before they are cut. The numbers are the
last thing to go.

## Windows

`0`-`7` or `--range` switch between lifetime totals and a window.

| key | window | starts at |
| --- | --- | --- |
| `0` | all time | lifetime totals |
| `1` | this session | panel start, no query |
| `2` | this week | Monday 00:00 local |
| `3` | last 7 days | rolling |
| `4` | this month | the 1st, local |
| `5` | last 30 days | rolling |
| `6` | this quarter | quarter start, local |
| `7` | this year | 1 January, local |

A window shows what you spent inside it, and swaps `SHARE` for `RATE`: dollars per day, and
days of balance left at that rate. The bar becomes what is left of the money that moved in the
window, `remaining / (spent + remaining)`. Lifetime keeps the purchase share.

Each window is one `POST /analytics/query` with exact instants, so it starts at local midnight,
not at UTC. Cached in memory for five minutes. The API has no top-up or past-balance data and
only reaches back 367 days, so a window never claims a starting balance.

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
