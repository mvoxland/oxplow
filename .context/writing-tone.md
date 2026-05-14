# Writing tone — blog posts & user docs

Applies to everything in `docs/` (the MkDocs site: blog posts, guides,
reference, philosophy) and any other reader-facing prose we ship —
release notes, changelogs, README copy.

Code comments and `.context/` docs are exempt; they're for ourselves
and follow different rules (see `CLAUDE.md`'s "default to writing no
comments" guidance).

The rules below are condensed from <https://tropes.fyi/tropes-md>.
They exist because LLM-generated prose has a recognisable shape: lots
of false drama, hedged grandiosity, and recycled scaffolding. Avoiding
that shape is most of what makes our writing readable.

## Meta-principle

Any one of these patterns used once may read fine. The damage comes
from stacking them or repeating one across a piece. **Vary, be
specific, accept some imperfection.** If a sentence feels like it
could open any blog post on any product, rewrite it.

## Word choice

- **No magic adverbs.** Drop "quietly", "deeply", "fundamentally",
  "remarkably", "arguably". They inflate mundane statements.
- **Banned vocabulary.** "delve", "utilize", "leverage" (verb),
  "robust", "streamline", "harness", "certainly". Use the plain word.
- **No grand nouns.** Avoid "tapestry", "landscape", "paradigm",
  "synergy", "ecosystem", "framework" as decoration. Name the actual
  thing.
- **Prefer "is" over "serves as".** "X is a reminder that…" beats
  "X serves as a reminder that…". Same for "stands as", "marks",
  "represents".

## Sentence structure

- **No "not X — it's Y" reframes.** Includes "not because X, but
  because Y" and "Not a bug. Not a feature. A design flaw." Just
  state Y.
- **No self-posed rhetorical questions.** "The result? Devastating."
  is a tell. If the reader wasn't asking, don't ask for them.
- **Don't repeat sentence openings.** "They assume… They assume…
  They assume…" reads like filler. Vary.
- **Use the rule of three sparingly.** One tricolon in a piece is
  fine; three in a row signals padding.
- **Cut filler transitions.** "It's worth noting", "Importantly",
  "Notably", "Interestingly", "bears mentioning". If the next
  sentence matters, just write it.
- **No `-ing` tail clauses for fake depth.** "…highlighting its
  importance", "…reflecting broader trends". Either say something
  substantive or end the sentence.
- **No fake ranges.** "From innovation to cultural transformation"
  isn't a spectrum. Only use "from X to Y" when X and Y sit on a
  real scale.

## Paragraph & list structure

- **Don't manufacture emphasis with one-sentence paragraphs.**
  "He published this. Openly. In a book." is theatrical. Vary length
  naturally.
- **If it's a list, format it as a list.** Don't disguise enumeration
  as prose with "The first… The second… The third…". Either write
  real connected paragraphs or use bullets.

## Tone

- **No "Here's the kicker" / "Here's the thing" / "Here's where it
  gets interesting".** Manufactured suspense before an unremarkable
  point.
- **No "Think of it like…" analogies by default.** They patronise
  technical readers. If a metaphor genuinely clarifies, use it once
  and move on.
- **No "Imagine a world where…"** openings.
- **No performative vulnerability.** "And honestly, I'll admit…"
  reads as polished, not honest. Real vulnerability is specific and
  uncomfortable.
- **Don't assert that something is obvious.** "The truth is simple",
  "History is clear", "The reality is…". Show the reader; don't
  announce.
- **Don't inflate stakes.** "Will fundamentally reshape", "define
  the next era", "something entirely new". Match the claim to the
  topic.
- **Don't lecture peers.** "Let's unpack this", "Let's dive in",
  "Let's break this down". Just say what you mean.
- **No vague attributions.** "Experts say", "industry reports
  suggest", "observers have noted". Name the source or cut the
  claim.
- **Don't invent jargon.** "The supervision paradox", "workload
  creep" used as if they're established terms. If you mint a phrase,
  define it; better, don't mint it.

## Formatting

- **Em dashes: ~2–3 per piece, max.** They're a strong tell when
  overused. Save them for genuine pauses.
- **Don't bold-prefix every bullet.** `**Thing**: description` on
  every line of a list looks like AI output. Use bolding when one
  item genuinely needs to stand out.
- **Plain ASCII.** `->` not `→`, straight quotes not curly,
  straight apostrophes. Match what a human types.

## Composition

- **Don't summarise at every level.** Intro that previews the piece,
  section recaps, conclusion that restates the intro — pick one,
  usually none. Trust the reader.
- **Introduce a metaphor once.** Don't run "ecosystem" or "engine"
  through every paragraph. If you find yourself reaching for the
  same image five times, the image isn't doing the work — the prose
  around it is.
- **Don't stack historical analogies.** "Apple didn't… Facebook
  didn't… Netflix didn't…" One example, if any.
- **Make the point once.** A 4000-word piece that says one 800-word
  thing five different ways is worse than the 800 words. Each
  paragraph should add information, not rephrase the prior one.
- **Never duplicate paragraphs.** Reread long pieces; AI-assisted
  drafts often repeat sections verbatim.
- **No signposted conclusions.** "In conclusion", "To sum up",
  "As we've seen". The structure should make the ending obvious.
- **No "Despite its challenges…" closer.** Acknowledging a problem
  only to dismiss it in the same sentence is a tell. Engage the
  tension or don't raise it.

## When you're editing

A practical pass after drafting: search the file for `delve`,
`tapestry`, `landscape`, `leverage`, `serves as`, `worth noting`,
`Imagine`, `In conclusion`, `Despite`, `—`, `→`. Each hit is a
question, not necessarily a fix — but each one earns its place or
gets cut.
