# hew

## What is it

hew reads structured log lines from stdin and reformats them according to
rules in `config.toml`. Each line is split into sections (timestamp, level,
message, ...) and each section can get a prefix/suffix based on a condition,
such as coloring by log level or flagging a keyword in the message.

## How to use it

Pipe logs into it:

```
./logs.sh | hew
```

`hew` reads its rules from `config.toml` in the working directory. A config
is a list of sections; each section pulls one attribute out of the input line
and applies the first affix whose condition matches.

### Example 1: color by level

Input config:

```toml
[[sections]]
name = "level"
attribute = "level"
affixes = [
    { condition = { Equals = "INFO" },  prefix = "[32m", suffix = "[0m" },
    { condition = { Equals = "DEBUG" }, prefix = "[34m", suffix = "[0m" },
    { condition = { Equals = "WARN" },  prefix = "[33m", suffix = "[0m" },
    { condition = { Equals = "ERROR" }, prefix = "[31m", suffix = "[0m" },
    { condition = "Always", prefix = " ", suffix = " " },
]
```

Output (INFO green, DEBUG blue, WARN yellow, ERROR red):

```
2026-08-13T19:28:03Z INFO request handled
2026-08-13T19:28:04Z ERROR user session expired
2026-08-13T19:28:10Z DEBUG user session expired
```

### Example 2: flag a keyword in the message

Input config:

```toml
[[sections]]
name = "message"
attribute = "message"
affixes = [
    { condition = { Regex = "(?i)69" }, prefix = "[35m69 Mentioned [0m ", suffix = "" },
    { condition = "Always", prefix = "", suffix = "" },
]
```

Output (any message containing "69" gets a marker prepended, everything else
passes through unchanged):

```
2026-08-13T19:28:05Z INFO 69 Mentioned -> user 69 requested a refund
2026-08-13T19:28:06Z INFO cache miss
```

### Example 3: wrap a field in brackets

Input config:

```toml
[[sections]]
name = "timestamp"
attribute = "timestamp"
affixes = [
    { condition = "Always", prefix = "[", suffix = "]" },
]
```

Output (the timestamp is always present, so the `Always` affix applies to
every line):

```
[2026-08-13T19:28:03Z] INFO request handled
[2026-08-13T19:28:04Z] ERROR user session expired
```

Sections stack: a real `config.toml` combines all three of these into one
pipeline, as the one shipped in this repo does.

## Compile it yourself

```
nix develop -c cargo build --release
```

The binary is at `target/release/hew`.
