# Metadata translator (parser) config syntax

comruvi fills the right-hand **Image information** pane from TOML translator
files. Each `.toml` file in `parserdir` is one translator. On startup comruvi
loads every `*.toml` in that directory, picks the best match for the selected
image, and renders the translator’s fields in order.

Default `parserdir` is `~/.config/comruvi/parsers`. Override it in
`~/.config/comruvi/config.toml`:

```toml
parserdir = "F:\\sync\\ast\\git\\comruvi\\parsers"
```

Shipped examples live next to this README. On first run they are copied into the
user parser directory; existing files are never overwritten.

---

## File overview

```toml
# Identity
name = "krea-ai"          # shown in the UI as "Parser: …"
priority = 100            # higher wins when several translators match
filetype = "png"          # png | jpg | jpeg | * (jpg also matches .jpeg)
chunk = "prompt"          # PNG text key that holds the ComfyUI node graph
builtin = "comfy_prompt"  # optional: run a built-in extractor after fields

# Selection — every [[trigger]] must match (AND)
[[trigger]]
node_class = "ConditioningKrea2Rebalance"

# Display — rendered top-to-bottom in this order
[[field]]
label = "Type"
constant = "Krea AI generated Image"

[[field]]
label = "Model Name"
node_class = "UNETLoader"
input = "unet_name"
```

Comments start with `#`. Keys are case-sensitive.

---

## Identity keys

| Key | Required | Meaning |
|-----|----------|---------|
| `name` | yes | Short id; shown as `Parser: <name>` |
| `priority` | no (default `0`) | Higher value is preferred when multiple translators match |
| `filetype` | no (default `*`) | `png`, `jpg` / `jpeg`, or `*` / `any` |
| `chunk` | no (default `prompt`) | PNG text-chunk key used as the ComfyUI JSON graph |
| `builtin` | no | After `[[field]]` rows, run a built-in helper (see below) |

---

## Triggers (`[[trigger]]`)

All trigger blocks in a file must succeed. A trigger needs at least one
condition. Conditions inside one block are AND.

| Key | Matches when |
|-----|----------------|
| `node_class` | A node in `chunk` has this exact `class_type` |
| `node_title` | That node’s `_meta.title` equals this string |
| `node_title_contains` | `_meta.title` contains this substring |
| `text_key` | A PNG text chunk with this key exists (e.g. `prompt`) |
| `exif` | An EXIF tag with this friendly name exists (e.g. `Model`) |

Examples:

```toml
# ComfyUI node class
[[trigger]]
node_class = "ConditioningKrea2Rebalance"

# Class + exact node title
[[trigger]]
node_class = "CLIPTextEncode"
node_title = "CLIP Text Encode (Positive Prompt)"

# Substring title match
[[trigger]]
node_class = "CLIPTextEncode"
node_title_contains = "Positive Prompt (Use natural language"

# PNG has a prompt chunk (generic ComfyUI)
[[trigger]]
text_key = "prompt"

# JPEG has a camera model in EXIF
[[trigger]]
exif = "Model"
```

Matching order: load all translators → keep those whose `filetype` fits → keep
those whose triggers all match → pick the highest `priority` (then `name`
alphabetically).

---

## Fields (`[[field]]`)

Each field becomes one row (or several rows for list extractors) in the info
pane. Fields are evaluated in file order. Empty results are skipped.

Every field needs a `label`. Then choose **one** value source:

### Constant

```toml
[[field]]
label = "Type"
constant = "Krea AI generated Image"
```

### Node input

Find every node matching the optional filters, read `inputs.<input>`:

```toml
[[field]]
label = "Model Name"
node_class = "UNETLoader"
input = "unet_name"

[[field]]
label = "Positive Prompt"
node_class = "CLIPTextEncode"
node_title = "CLIP Text Encode (Prompt)"
input = "text"
```

Optional filters on a field (same meaning as on triggers):

- `node_class`
- `node_title`
- `node_title_contains`

### Extract helpers

For structured LoRA nodes, set `extract` instead of `input`:

| `extract` | Expects | Output rows |
|-----------|---------|-------------|
| `lora_loader` | `lora_name`, `strength_model` | `name (weight …; active)` |
| `lora_manager` | `loras` array or `loras.__value__` | name, model/CLIP strengths, active/inactive |
| `power_lora` | `lora_1`, `lora_2`, … with `{ on, lora, strength }` | Power Lora Loader (rgthree) |
| `lora_stack` | `lora_01` / `strength_01`, … | Lora Loader Stack (rgthree); skips `"None"` |

```toml
[[field]]
label = "LoRA"
node_class = "Power Lora Loader (rgthree)"
extract = "power_lora"

[[field]]
label = "LoRA"
node_class = "Lora Loader (LoraManager)"
extract = "lora_manager"
```

Rows labeled `LoRA` (or `LoRA List`) are grouped under **LoRA list** in the UI.
Values containing `; inactive)` are shown in gray.

### EXIF tag

```toml
[[field]]
label = "Kamera"
exif = "Model"
```

Supported friendly names include: `Make`, `Model`, `LensModel`, `LensMake`,
`ExposureTime`, `FNumber`, `ISO`, `PhotographicSensitivity`, `ISOSpeedRatings`,
`FocalLength`, `DateTimeOriginal`, `DateTime`, `Orientation`.

---

## Built-in helpers (`builtin`)

| Value | Behavior |
|-------|----------|
| `comfy_prompt` | Walk the prompt graph: model, prompts via KSampler links when titles are missing, sampler params, and any LoRA-like nodes |

`builtin` runs **after** the explicit `[[field]]` list. Use it on a low-priority
fallback translator (see `comfyui-generic.toml`).

---

## Practical tips

1. Prefer a **unique** trigger (a rare `class_type` or a distinctive `_meta.title`).
2. Give specialist translators a **higher `priority`** than the generic fallback.
3. You can list several `[[field]]` blocks with the same `label` (e.g. two LoRA
   extractors); all non-empty results are shown.
4. Restart comruvi after editing translator files (they are loaded at startup).
5. The info pane shows `Parser: <name>` so you can confirm which file matched.

---

## Minimal photo example

```toml
name = "photo"
priority = 50
filetype = "jpg"

[[trigger]]
exif = "Model"

[[field]]
label = "Type"
constant = "Photo"

[[field]]
label = "Kamera"
exif = "Model"

[[field]]
label = "Shutter Speed"
exif = "ExposureTime"
```

## Minimal ComfyUI example

```toml
name = "my-workflow"
priority = 70
filetype = "png"
chunk = "prompt"

[[trigger]]
node_class = "CLIPTextEncode"
node_title = "CLIP Text Encode (Positive Prompt)"

[[field]]
label = "Type"
constant = "My workflow"

[[field]]
label = "Positive Prompt"
node_class = "CLIPTextEncode"
node_title = "CLIP Text Encode (Positive Prompt)"
input = "text"
```
