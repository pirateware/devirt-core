#!/usr/bin/env bash
#
# Pull the obfuscated-JS corpus from the Hugging Face dataset into ./samples.
#
#   scripts/pull-corpus.sh            # clone or fast-forward ./samples
#   scripts/pull-corpus.sh <dir>      # into a different directory
#   CORPUS_REV=<sha> scripts/pull-corpus.sh   # pin a revision (reproducible scoring)
#
# The corpus lives OUTSIDE this repo; ./samples is gitignored. HF datasets are
# plain git repos, so no huggingface-cli/Python is needed.
set -euo pipefail

REPO="${CORPUS_REPO:-https://huggingface.co/datasets/devirt-dev/devirt-corpus}"
DEST="${1:-samples}"

if [ -d "$DEST/.git" ]; then
  echo "Updating corpus in $DEST ..."
  git -C "$DEST" fetch --quiet origin
  git -C "$DEST" checkout --quiet "${CORPUS_REV:-origin/HEAD}"
else
  echo "Cloning $REPO into $DEST ..."
  git clone --quiet "$REPO" "$DEST"
  [ -n "${CORPUS_REV:-}" ] && git -C "$DEST" checkout --quiet "$CORPUS_REV"
fi

echo "Done: $(find "$DEST" -name '*.js' 2>/dev/null | wc -l | tr -d ' ') .js files in $DEST/"
