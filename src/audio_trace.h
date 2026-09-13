#ifndef KFX_AUDIO_TRACE_H
#define KFX_AUDIO_TRACE_H

#ifdef __cplusplus
extern "C" {
#endif

void sound_trace_event(const char *event, long emitter, long requested, long resolved,
    long voice, long volume, long pan, long pitch, long repeats, long priority);

#ifdef __cplusplus
}
#endif
#endif
