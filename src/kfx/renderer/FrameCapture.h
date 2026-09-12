#pragma once

struct SDL_Surface;

void CaptureFrameIfRequested(SDL_Surface* indexed, SDL_Surface* rgba);
