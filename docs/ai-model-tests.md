# AI model tests

How we test the AI models we suggest (in the server's admin pages and on
xenify.net), what we found, and what is still to fix before any result is
published. Started 2026-10-25.

## The text test

`xenify-wp-corpus bench --doc 40 --docs 10 --models <ids>` builds 10
documents of 40 test passages (about 630 words each): about half have one
mistake, the rest are correct standard or informal English. Each document is
sent whole with Write's own AI check prompt (`xenify_wp_ai::review_request`,
grammar focus), the way Write sends it. `--url`/`--key-env` point it at any
OpenAI-style address (LocalAI on a rented GPU) instead of OpenRouter. Every
reply is saved beside the results (`.replies.jsonl`);
`bench --doc 40 --docs 6 --samples FILE.replies.jsonl` lists false alarms,
fixes that differ from ours, and misses, from the saved replies (no new
requests).

Run at most about 10 models at once: 340 requests at once made this
container's address lookups fail.

## Findings (2026-10-25)

Partial run of 34 models through OpenRouter (zero-data-retention providers):
results in `data/proofing-corpus/doc40/`. **Not publishable yet**: see
"Scoring is wrong" below.

- **Too slow.** One 630-word document took 6–110 s even on OpenRouter; a home
  graphics card is slower. Most of the time is the model writing up to 25
  issues with a sentence of explanation each, plus hidden "thinking" for the
  models that think. The user called it unusable, local and outside alike.
- **Scoring is wrong.** "Right fix" only accepts our exact corrected sentence,
  so correct fixes count as wrong: "this are" → "these are" (ours: "this
  is"), "more guiltier" → "more guilty", "fantastic. I got" (ours: a
  semicolon), "weekend; organizers" (ours: ", and organizers"). Published
  rates must not call a correct fix wrong.
- **The test data has mistakes.** Some "correct" passages have real errors
  ("I can't believe how fast she can run, she just won …" is a comma splice),
  and some expected fixes are one choice among several ("the band … they" →
  "it", "forbid him from going", "helped the team to collect").
- **Informal writing.** Write's prompt never says to keep the writer's own
  style, so models "fix" slang and dialect. DeepSeek V4 Flash rewrites casual
  text to sound formal ("inappropriate for professional writing") and once
  reported a correct sentence with the fix "No change needed — this one is
  correct." Qwen3.8 27B flags dialect but says it is likely intentional.
  Most of DeepSeek V4 Flash's extra false alarms are this, not grammar.
- **Write places short fixes wrongly.** `find_quote` takes the first match in
  the paragraph: a model's correct "i" → "I" in "My wife had the salmon and i
  had the steak" lands on the "i" in "wife". Anchor fixes near where the
  mistake is (quote with context, or the closest match to the reported
  position).
- Models whose providers can't fill in Write's tool (Gemma 3 4B, Phi-4, …)
  answer when asked for the same JSON in plain text (the bench now does this;
  Write doesn't).

## Fair scoring (done 2026-10-25)

`bench --doc 40 --docs 10 --judge DIR` scores every saved run in `DIR` again,
asking two judge models from different makers (default Qwen3 235B and
gpt-oss-120b); verdicts are cached in `DIR/judged.jsonl`, so rescoring is free.
It writes `DIR/judged.md` (the table) and `DIR/review.md` (what a person
decides: the judges disagreed, and test passages left out).

- A **fix is right** when it gives our sentence, or both judges say it is
  correct English with the same meaning that changes only what was wrong.
- **Found, no fix offered**: the model marked the mistake but gave only a
  note. Counted for finding, not fixing (it isn't a wrong fix).
- A flag on a correct sentence is a **false alarm** only when both judges say
  the marked words are fine.
- **Test passages** count only when both judges confirm them (a mistake really
  is one and our fix is right; a correct sentence really is). The first run
  left out 45 of 350, among them real comma splices that were filed as
  correct; the judges are strict, so a few fine sentences go too (that only
  makes the test smaller, never a model's score lower).
- **Empty replies** are left out and shown as "Answered n/10": they are a
  provider failing, not the model being wrong. Models with only a few answered
  documents need a rerun before anything is published about them.
- Hand check of a sample (2026-10-25): accepted fixes were right ("don't work"
  → "doesn't", "on accident" → "accidentally"); one accepted fix also
  lower-cased the sentence's first word. Rejected "fixes" were mostly notes
  without a fix, now counted apart.

## Write's AI check made faster and fairer (2026-10-25)

Changes in `xenify-wp-ai` (used by Write and by the bench):

- The review prompt keeps the writer's own voice (slang, dialect, texting
  left alone unless the wanted tone asks for formal writing), says optional
  choices are not mistakes (comma after "Yesterday", the Oxford comma, "data
  was/were", "log-in/login"), asks to be sure before reporting and for
  explanations of a few words.
- Issues whose fix is the same words or says "no change"/"is correct" are
  dropped (`parse_review`).
- Quotes are matched on whole words first ("i" is the word, not the one in
  "wife").
- `review_parts`: Write sends the document in parts of about 250 words, all
  at once, and merges the answers; `without_tools`: models whose providers
  can't fill in the tool are asked for the same JSON in plain text.

Measured through OpenRouter on 10 documents of about 630 words, judged (see
`data/proofing-corpus/v2-*/`). Seconds per document · false alarms ·
informal writing changed:

| Model | Old prompt, whole | New prompt, whole | New, in parts | New, parts, thinking off |
|---|---|---|---|---|
| Gemma 4 26B-A4B | 20 s · 2% · 44% | 11 s · 0% · 6% | 7 s · 0% · 4% | 7 s · 0% · 4% |
| Gemma 4 31B | 20 s · 2% · 18% | 19 s · 0% · 0% | 14 s · 0% · 0% | 12 s · 0% · 0% |
| DeepSeek V4 Flash | 20 s · 9% · 36% | 18 s · 3% · 10% | 12 s · 2% · 14% | 9 s · 2% · 18% |
| Qwen3.8 27B | 37 s · 2% · 26% | 55 s · 1% · 2% | 38 s · 0% · 0% | 9 s · 2% · 12% |

All four still find 89–98% with 96–100% right fixes. Qwen3.5 9B thinks for
minutes per page through OpenRouter (60 s even with thinking off): not usable
for Write's check that way. gpt-oss-20b's providers mostly failed.

Still open: thinking off isn't in Write yet (it helps Qwen a lot, costs it a
little in informal writing). On LocalAI it matters even more: Gemma 4 26B-A4B
thinks by default there and took 284 s on one 250-word part (A6000). LocalAI
ignores OpenRouter's `reasoning: {enabled: false}`, `think: false` and
`chat_template_kwargs`; only `reasoning_effort: "none"` turns it off (a short
fix went from 6 s to 1.6 s). Results showing as each part arrives (Write
shows them all at the end); only sending changed paragraphs.

## LocalAI as Write needs it (found on a rented RTX 3080 Ti, 2026-10-25)

Things the server and the Hub's LocalAI part must set, found while running
`scripts/pod-bench/job.py`:

- **Engines download slowly from quay.io.** LocalAI fetches each engine
  (llama.cpp 1.8 GB, diffusers 4.6 GB, …) from quay.io first; on the pod that
  ran at about 0.35 MB/s (1.5 hours for llama.cpp) while Hugging Face gave
  about 20 MB/s. On a second machine (RTX A6000) quay.io gave about 20 MB/s,
  so it depends on the machine. The same engines are on Docker Hub
  (`localai/localai-backends:…`), but blocking quay.io doesn't make LocalAI
  v4.12.1 use them: it only tries other quay.io tags, then the install fails.
  The Hub could install engines from Docker Hub itself, or show progress
  honestly so nobody thinks it's stuck.
- **`/etc/hosts` in a container is a mounted file:** edit it in place
  (`sed -i` fails with "Device or resource busy").
- **Parallel requests.** Write sends a document's parts at once. Started with
  one-model-at-a-time (`LOCALAI_SINGLE_ACTIVE_BACKEND`), LocalAI deadlocked:
  it tried to unload the busy model to make room for the second request. Run
  it with `LOCALAI_PARALLEL_REQUESTS=true` and llama.cpp slots
  (`LLAMACPP_PARALLEL=4`).
- **Context size.** LocalAI splits a model's context between its parallel
  slots, and catalogue entries are small: a 250-word part plus Write's prompt
  and answer overflowed ("Context size has been exceeded"). When the server
  installs a text model for Write it should set `context_size` (16384 for 4
  slots).
- **Engine builds follow the CUDA LocalAI detects.** On the A6000 pod it
  detected CUDA 12 (the image's `CUDA_VERSION=12.8.1`, although the driver
  does 13). The vllm-cpp engine (MiniMax H3's) only has a CUDA 13 build for
  NVIDIA, so LocalAI installed the processor build and the model refused to
  load ("no accelerator backend is registered in this build").
  `LOCALAI_FORCE_META_BACKEND_CAPABILITY=nvidia` makes it pick the card's
  build. The Hub should set this from the driver it finds.
- **Z-Image Turbo's catalogue entry burns its pictures.** It says
  `cfg_scale: 0`; the diffusers engine reads 0 as unset and uses 7, which
  overcooks a turbo model (blurry, washed out). Set `cfg_scale: 1` after
  installing.
- **FLUX.1 schnell needs a Hugging Face account.** Its repository is gated
  (accept the terms, then a token), so LocalAI's install gets 401. FLUX.2
  klein 4B (Apache 2.0, public files in LocalAI's catalogue) is the FLUX to
  offer in Add models.
- **Wan 2.2 can't run on LocalAI 4.12.1.** Both its engines (cuda12- and
  cuda13-vllm-omni) ship PyTorch `2.9.1+cpu`; swapping in the CUDA build then
  fails because vLLM-Omni needs a newer vLLM than the one packaged.
- **MiniMax H3 (vllm-cpp) didn't finish a 5-second clip in 98 minutes.** The
  catalogue entry says "loads, not rendered". vllm-cpp also needs ffmpeg on
  the machine to put the frames into a video.
- **Deleting a model leaves its downloads** in `models/.artifacts` (149 GB
  after a few models): the Hub's delete must clear them too.

## GPU results (RTX A6000 48 GB, 2026-10-25)

Text on LocalAI, thinking off, judged:

| Model | Per 1,000 words | Found | Right fix | False alarms | Informal changed |
|---|---:|---:|---:|---:|---:|
| Gemma 4 26B-A4B | 13 s | 93% | 98% | 1% | 4% |
| Gemma 4 12B | 24 s | 87% | 93% | 0% | 4% |
| Qwen3.8 27B | 25 s | 84% | 98% | 2% | 20% |
| Qwen3.5 9B | 24 s | 65% | 96% | 7% | 34% |

Thinking on: minutes per document, most timed out.

Pictures (LocalAI): FLUX.2 klein 4B 4 s at 1024², 6 s at 1536×1024, 1.3 s
at 512² once loaded; Z-Image Turbo (after the cfg_scale fix) 15 s, 20–24 s,
3.6 s. FLUX.2 klein follows odd prompts better; Z-Image is more photo-like.

Video, 5-second clips: Wan 2.2 14B in ComfyUI's default template (640×640,
81 frames, 20 steps) 710 s; MiniMax H3 in ComfyUI's default template
(864×480, 124 frames with sound, 20 steps) 390–430 s; Wan 2.1 1.3B in LocalAI
291 s at 832×480 and 1,192 s at 1280×720, rough quality. ComfyUI set up and
downloaded 80 GB in under 3 minutes and ran every template first time.

## GPU results (RTX 3090 24 GB and Tesla V100 16 GB, 2026-10-25/26)

Pictures and video through ComfyUI's API, the way the server's ComfyUI
connection will use it, with an untimed warm-up first so times exclude
loading; text on LocalAI and Ollama. Seconds per document (about 630 words),
thinking off; scores judged as above.

| Model | 3090 LocalAI | 3090 Ollama | V100 LocalAI | V100 Ollama | V100 found / right fix / false alarms |
|---|---:|---:|---:|---:|---|
| Gemma 4 26B-A4B | 7 s | 6 s | crashes | 56 s | 95% / 95% / 0% (Ollama) |
| Gemma 4 12B | 13 s | 7 s | 19 s | 13 s | 83–85% / 91–96% / 0% |
| Qwen3.8 27B | 13 s | 16 s | crashes | over 180 s (timed out) | — |
| Qwen3.5 9B | 12 s | 18 s | 19 s | 52 s, notes without fixes | 75% / 91% / 0% (LocalAI) |

- **Models bigger than the card:** LocalAI sets `n_gpu_layers=99`, so a model
  that doesn't fit crashes the llama.cpp backend (no answer at all). Ollama
  keeps the rest in system memory: Gemma 26B (mixture of experts) stays
  usable at 56 s, dense Qwen3.8 27B (32% on CPU) does not. The server should
  warn, or lower the GPU layers, when a model is bigger than the card.
- **On a 16 GB card** Gemma 4 12B is the pick for Write (13 s, no false alarms).
- **Volta (V100, compute 7.0):** PyTorch's cu128 builds leave it out; the
  cu126 build works with driver 550. MiniMax's nvfp4 text encoder loads.
- **Judging:** the second judge timed out often on OpenRouter; three runs
  left 119–129 of 350 passages unconfirmed (29–43 on the 3090 run), scores
  stable between runs.

Pictures (ComfyUI), 1024² / 1536×1024 / 1024×1536: FLUX.2 klein 4B 3.1 / 5.1
/ 5.0 s on the A6000 (re-run 2026-10-26, pictures only), 4.0 / 6.0 / 6.0 s on
the 3090, 4.0 / 6.5 / 7.0 s on the V100; Z-Image Turbo 6.1 / 9.0 / 9.1 s,
9.0 / 13.0 / 13.1 s and 9.0 / 16.0 / 16.0 s. LocalAI's Z-Image on the A6000
was 2.4–2.7× slower than ComfyUI's on the same card.

Video, 5-second clips, 20 steps, same size on every card: MiniMax H3 (864×480,
124 frames, sound) 7.5–8.5 min on the 3090, 17.5 min on the V100; Wan 2.2 14B
(640×640, 81 frames) 20 min on the 3090, 29.2 min on the V100. (The V100
showcase clips were first made at 832×480: 16.5 and 27.7 min.)

Every V100 picture and clip matches its prompt, no logos. Wan 2.2 drew the
moustache as a beard and the baby doesn't eat; MiniMax followed both clips
closely.

## What the server uses (decided 2026-10-26, built)

People install the AI programs themselves, from each program's own guide;
Xenify only points to them and says how to connect (xenify.net/download/#ai).

- **Text: Ollama**, recommended and the default on the server's AI page. It
  was faster than LocalAI with the recommended Gemma models and keeps
  working when a model is a little too big for the card (the rest runs from
  the computer's memory); LocalAI crashes then. LocalAI, LM Studio and any
  OpenAI-style service still work.
- **Pictures and clips: ComfyUI** (`crates/xenify-server/src/comfy.rs`),
  installed with Stability Matrix (which also finds and downloads models)
  or ComfyUI's own installer. The admin types its address on the AI page.
  The server knows the four models tested above and runs each with exactly
  the workflow measured here: FLUX.2 klein 4B (4 steps, CFG 1) and Z-Image
  Turbo for pictures, Wan 2.2 14B (640x640, 81 frames) and MiniMax H3
  (864x480, 124 frames with sound) for clips. The workflows are ComfyUI's
  own templates (`crates/xenify-server/comfy/`, from Comfy-Org/workflow_templates,
  MIT) with the person's words, the size and a new seed put in.
- The server asks ComfyUI which files it has (`/object_info` for the UNET,
  text encoder, VAE and LoRA loaders) and shows, for each model, "ready" or
  the files still missing with where to download them. It never downloads
  into ComfyUI itself. Ready models can be offered to people like any other.
- Apps ask exactly as before (`/v1/ai/images/generations`, `/v1/ai/video`,
  answered as `{"data": [{"b64_json": …}]}`): a request whose model is one of
  ComfyUI's goes to ComfyUI (`/prompt`, `/history`, `/view`), anything else
  to the text program. Clips wait up to an hour (about half an hour on a
  16 GB card).
- Not yet: picture editing through ComfyUI. Draw's "AI › Change this picture"
  sends edits to `POST /v1/ai/chat/completions` with the server's picture
  model, as an OpenRouter-style request (`xenify_openrouter::images::edit_request`):
  text, the picture, optionally a mask (white = change) and reference
  pictures as `image_url` parts, `modalities: ["image","text"]`; it reads
  `choices[0].message.images[0].image_url.url` (data URL) and
  `choices[0].message.content`. An edit workflow (Qwen-Image-Edit or FLUX
  Kontext) answering in that shape would make it work; Draw blends the result
  only inside the mask. It needs its own test run first.
- MiniMax H3's licence doesn't allow running it, or showing its clips, in
  the US, the EU, the UK or South Korea. The server offers it like the others;
  the setup guide and the AI page say so.

## To fix, in order

1. ~~**Scoring.**~~ Done (above). A fix counts as right when the corrected sentence is ours, or
   when two judge models from different makers agree it is correct English,
   keeps the meaning and changes only what was wrong. Disagreements are
   listed for a person to decide. The same judges check each "false alarm" on
   a correct passage: when they say it is a real mistake, the test passage is
   fixed instead. Hand-check a sample of the judges' verdicts and publish how
   often they agreed with the person.
2. **Test data.** Fix or drop the passages found above; report informal
   writing separately ("changed informal writing") from false alarms on
   standard English.
3. **Write's AI check.** Keep the writer's style unless the document's tone
   asks otherwise; anchor fixes; ask models without tool support for plain
   JSON; make it fast (thinking off, short answers, a few paragraphs per
   request in parallel with results as they arrive, only changed paragraphs,
   spelling left to the built-in checker).
4. **Run again**, then the GPU timings (`scripts/gpu-bench.py`, RunPod, no
   network volume: download per card).
5. **Publish** per model: found, right fix, false alarms, informal changed,
   seconds per page, card size needed. On the admin page's Add models list and
   on xenify.net's Server page, with the date and how it was measured.

## Notes on models

Shown with a model's results (admin page, xenify.net) when they explain a
number.

### DeepSeek V4 Flash

Finds and fixes as well as the best models (96% found, 97% right fix in the
first judged run) but raises more false alarms (9%, against 2% for the best).
They come from how it answers, not from not knowing grammar. Its 14 false
alarms that both judges confirmed (2026-10-25), by kind:

- **Reports a correct sentence and says so itself.** "After finishing my
  degree, I completed a one-year internship at Siemens." → fix "No change
  needed — this one is correct." (note: "Missing comma after introductory
  phrase", though the comma is there). The same for "Out of all the
  moisturizers I've tried, this one…" and "You're welcome, Jenna." It seems to
  start an issue while still thinking and report it anyway.
- **Optional commas as mistakes.** "Yesterday I saw a beautiful rainbow" →
  "Yesterday, I saw…" (three times); "a playground, walking paths and a
  community garden" → Oxford comma; "The phone arrived yesterday and I already
  set it up" → comma before "and"; "Listen, James, the control group…" → a
  colon.
- **Muddled answers.** "I need to nip it in the bud before the glue fully
  sets" → "fix it" (called the wrong idiom), plus a second issue on the same
  sentence about a missing comma before "but" (there is no "but").
  "Neither of the two theories … explains": its own note says "should be
  'explain'? Actually … singular is more formal and correct here", and it
  reported it anyway. "what the three things we fear most are": "should be
  'what the three things we fear most are', not 'what the three things we
  fear most are'".
- **A spelling preference.** "log-in credentials" → "login".

For comparison, Qwen3.8 27B's 2 confirmed false alarms were pedantic but
sensible: "the data was" → "were", and moving "is" to the end of "what the
best way is to get to the stadium".

What Write can do about it, for every model: drop an issue whose fix is the
same words or says "no change", and tell the model that optional commas
(after "Yesterday", the Oxford comma) are not mistakes.
