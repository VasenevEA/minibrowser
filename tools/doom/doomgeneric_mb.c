// doomgeneric for MiniBrowser: compiled to plain JavaScript (no WebAssembly),
// it runs inside the page's JS host. Frames go to a <canvas> through
// putImageData, keys come from the page (window.mbDoomKey), time from the
// page clock; the page's requestAnimationFrame drives the game loop.

#include "doomkeys.h"
#include "doomgeneric.h"

#include <stdint.h>
#include <stdlib.h>
#include <emscripten.h>

static uint8_t *rgba;

// The page shows the frame: RGBA bytes at `ptr`, w x h.
EM_JS(void, mb_frame, (uint8_t *ptr, int w, int h), {
  if (typeof Module.mbFrame === 'function') Module.mbFrame(HEAPU8.subarray(ptr, ptr + w * h * 4), w, h);
});

// Next key event from the page: (pressed << 8 | doom key), or -1.
EM_JS(int, mb_key, (void), {
  var q = Module.mbKeys;
  return q && q.length ? q.shift() : -1;
});

// Real time: the game waits in a loop for the next tic, so the clock must move
// while it runs (the page clock stands still during a script).
EM_JS(double, mb_now, (void), {
  return Date.now();
});

void DG_Init(void)
{
    rgba = malloc(DOOMGENERIC_RESX * DOOMGENERIC_RESY * 4);
}

void DG_DrawFrame(void)
{
    // DG_ScreenBuffer is 0x00RRGGBB per pixel.
    const uint32_t *src = DG_ScreenBuffer;
    uint8_t *d = rgba;
    for (int i = 0; i < DOOMGENERIC_RESX * DOOMGENERIC_RESY; i++) {
        uint32_t p = src[i];
        d[0] = p >> 16;
        d[1] = p >> 8;
        d[2] = p;
        d[3] = 255;
        d += 4;
    }
    mb_frame(rgba, DOOMGENERIC_RESX, DOOMGENERIC_RESY);
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
    doomgeneric_Tick();
}

int main(int argc, char **argv)
{
    doomgeneric_Create(argc, argv);
    emscripten_set_main_loop(tick, 0, 1);
    return 0;
}
