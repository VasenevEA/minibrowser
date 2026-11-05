// doomgeneric for MiniBrowser: compiled to plain JavaScript (no WebAssembly),
// it runs inside the page's JS host. Frames go to a <canvas> through
// putImageData, keys come from the page (window.mbDoomKey), time from the
// page clock; the page's requestAnimationFrame drives the game loop.

#include "doomkeys.h"
#include "doomgeneric.h"

#include <stdint.h>
#include <stdlib.h>
#include <emscripten.h>

#include "i_video.h"

// Built with CMAP256: DG_ScreenBuffer holds palette indices, `colors` the palette.
extern struct color colors[256];
extern boolean palette_changed;

static uint32_t *rgba;
static uint32_t pal[256];

// The page shows the frame: RGBA bytes at `ptr`, w x h.
EM_JS(void, mb_frame, (uint8_t *ptr, int w, int h), {
  if (typeof Module.mbFrame === 'function') Module.mbFrame(HEAPU8.subarray(ptr, ptr + w * h * 4), w, h);
});

// Next key event from the page: (pressed << 8 | doom key), or -1.
EM_JS(int, mb_key, (void), {
  var q = Module.mbKeys;
  return q && q.length ? q.shift() : -1;
});

// Game time: real time (the game waits in a loop for the next tic, so the
// clock must move while it runs; the page clock stands still during a script),
// less the time beyond `Module.mbMaxStep` ms between frames, if set. On a slow
// device the game then runs slower instead of catching up many tics of logic
// before each frame: more frames, in slow motion.
EM_JS(double, mb_now, (void), {
  return Date.now() - (Module.mbLag || 0);
});

EM_JS(void, mb_frame_start, (void), {
  var now = Date.now(), max = Module.mbMaxStep;
  if (max && Module.mbLast && now - Module.mbLast > max) Module.mbLag = (Module.mbLag || 0) + (now - Module.mbLast - max);
  Module.mbLast = now;
});

void DG_Init(void)
{
    rgba = malloc(DOOMGENERIC_RESX * DOOMGENERIC_RESY * 4);
    palette_changed = true;
}

void DG_DrawFrame(void)
{
    // One pass, one store per pixel: the loop runs interpreted on the device.
    if (palette_changed) {
        for (int i = 0; i < 256; i++)
            pal[i] = 0xff000000u | (uint32_t)colors[i].b << 16 | (uint32_t)colors[i].g << 8 | colors[i].r;
        palette_changed = false;
    }
    const uint8_t *src = (const uint8_t *)DG_ScreenBuffer;
    for (int i = 0; i < DOOMGENERIC_RESX * DOOMGENERIC_RESY; i++)
        rgba[i] = pal[src[i]];
    mb_frame((uint8_t *)rgba, DOOMGENERIC_RESX, DOOMGENERIC_RESY);
}

void DG_SleepMs(uint32_t ms)
{
    (void)ms; // the page's frame loop paces the game
}

uint32_t DG_GetTicksMs(void)
{
    return (uint32_t)mb_now();
}

int DG_GetKey(int *pressed, unsigned char *key)
{
    int k = mb_key();
    if (k < 0)
        return 0;
    *pressed = (k >> 8) & 1;
    *key = k & 0xff;
    return 1;
}

void DG_SetWindowTitle(const char *title)
{
    (void)title;
}

static void tick(void)
{
    mb_frame_start();
    doomgeneric_Tick();
}

int main(int argc, char **argv)
{
    doomgeneric_Create(argc, argv);
    emscripten_set_main_loop(tick, 0, 1);
    return 0;
}
