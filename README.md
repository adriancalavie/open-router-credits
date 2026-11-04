# open-router-credits

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
This tool answers one question: "what is left?"

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
open-router-credits --setup           # change the key
open-router-credits --once            # print the values and exit
open-router-credits --config <PATH>   # use another config file
```

Press `q` to quit, `r` to refresh, and `?` for help. It needs 60 columns and 12 rows.

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
