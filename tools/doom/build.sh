#!/bin/sh
# Builds Doom (doomgeneric) as plain JavaScript for www/doom: emscripten with
# wasm2js (QuickJS has no WebAssembly). Needs emcc; fetches doomgeneric and the
# shareware doom1.wad.
set -e
here=$(cd "$(dirname "$0")" && pwd)
work=${WORK:-$HOME/dev/mb-doom}
mkdir -p "$work" && cd "$work"
[ -d doomgeneric ] || git clone --depth 1 https://github.com/ozkl/doomgeneric.git
[ -f doom1.wad ] || curl -sL -o doom1.wad https://cdn.jsdelivr.net/gh/Akbar30Bill/DOOM_wads@master/doom1.wad
echo "5b2e249b9c5133ec987b3ea77596381dc0d6bc1d  doom1.wad" | shasum -c -
cp "$here/doomgeneric_mb.c" doomgeneric/doomgeneric/
cd doomgeneric/doomgeneric
src=$(grep "^SRC_DOOM" Makefile.emscripten | sed 's/SRC_DOOM = //; s/doomgeneric_emscripten.o/doomgeneric_mb.o/; s/ i_sdlmusic.o//; s/ i_sdlsound.o//; s/ mus2mid.o//; s/\.o/.c/g')
emcc -O2 -DCMAP256 -DDOOMGENERIC_RESX=320 -DDOOMGENERIC_RESY=200 -w $src -o "$here/../../www/doom/doom.js" \
  -sWASM=0 -sENVIRONMENT=shell -sINITIAL_MEMORY=16777216 -sSTACK_SIZE=1048576 -sEXIT_RUNTIME=0 \
  -sFORCE_FILESYSTEM=1 -sEXPORTED_RUNTIME_METHODS=FS
cp "$work/doom1.wad" "$here/../../www/doom/"
