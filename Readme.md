# jclean

Pretty-print JSON files — even messy or broken ones.

*jclean* — **J**SON **clean** read.

## Build

```sh
cargo build --release
```

## Usage

```sh
jclean file.json
cat file.json | jclean -
```

| Option | |
|---|---|
| `-v`, `--verbose` | print parsing info on stderr |
| `-i`, `--indent N` | indentation width (default: 2) |
| `--color` / `--no-color` | force / disable colors (`NO_COLOR` is honored) |

## What it reads

Valid JSON first. If that fails, a lenient parser accepts:

- unquoted keys: `{ apps: [...] }`
- `'single'` and `` `backtick` `` strings
- `//`, `/* */` and `#` comments
- extra or missing commas
- JS numbers: `0x1F`, `.5`, `+1`, `1_000`
- `True`, `None`, `undefined`, unquoted words
- `module.exports = { ... };` (e.g. PM2 `ecosystem.config.js`)

Several documents in a row (NDJSON) are supported. Output is always valid JSON, key order preserved.

## Example

```sh
$ echo "{ name: 'api', ports: [80, 443,], }" | jclean -
{
  "name": "api",
  "ports": [
    80,
    443
  ]
}
```
