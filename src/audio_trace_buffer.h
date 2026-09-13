#ifndef KFX_AUDIO_TRACE_BUFFER_H
#define KFX_AUDIO_TRACE_BUFFER_H

#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>

#ifndef KFX_AUDIO_TRACE_CAPACITY
#define KFX_AUDIO_TRACE_CAPACITY 65536
#endif

struct AudioTraceRecord {
    const char *event;
    uint64_t tick, turn;
    uint32_t sound_seed;
    int64_t emitter, requested, resolved, voice, volume, pan, pitch, repeats, priority;
};

static struct AudioTraceRecord *audio_trace_records;
static size_t audio_trace_count;
static uint64_t audio_trace_overflow;
static FILE *audio_trace_file;

static void audio_trace_flush(void)
{
    if (!audio_trace_file) return;
    for (size_t i = 0; i < audio_trace_count; ++i) {
        const struct AudioTraceRecord *r = &audio_trace_records[i];
        fprintf(audio_trace_file,
            "{\"event\":\"%s\",\"sequence\":%zu,\"tick\":%" PRIu64 ",\"turn\":%" PRIu64
            ",\"sound_seed\":%" PRIu32 ",\"emitter\":%" PRId64 ",\"requested\":%" PRId64
            ",\"resolved\":%" PRId64 ",\"voice\":%" PRId64 ",\"volume\":%" PRId64
            ",\"pan\":%" PRId64 ",\"pitch\":%" PRId64 ",\"repeats\":%" PRId64
            ",\"priority\":%" PRId64 "}\n",
            r->event, i, r->tick, r->turn, r->sound_seed, r->emitter, r->requested,
            r->resolved, r->voice, r->volume, r->pan, r->pitch, r->repeats, r->priority);
    }
    fprintf(audio_trace_file, "{\"event\":\"trace_end\",\"records\":%zu,\"overflow\":%" PRIu64 "}\n",
        audio_trace_count, audio_trace_overflow);
    fclose(audio_trace_file);
    audio_trace_file = NULL;
    free(audio_trace_records);
    audio_trace_records = NULL;
}

static void audio_trace_init(void)
{
    static int initialized;
    if (initialized) return;
    initialized = 1;
    const char *path = getenv("KFX_AUDIO_TRACE");
    if (!path || !*path) return;
    audio_trace_records = (struct AudioTraceRecord *)calloc(KFX_AUDIO_TRACE_CAPACITY, sizeof(*audio_trace_records));
    if (!audio_trace_records) return;
    audio_trace_file = fopen(path, "wb");
    if (!audio_trace_file) {
        free(audio_trace_records);
        audio_trace_records = NULL;
        return;
    }
    if (atexit(audio_trace_flush) != 0) {
        fclose(audio_trace_file);
        audio_trace_file = NULL;
        free(audio_trace_records);
        audio_trace_records = NULL;
        return;
    }
    fprintf(audio_trace_file, "{\"event\":\"trace_begin\",\"format\":\"KFXAUDIO1\",\"capacity\":%d}\n", KFX_AUDIO_TRACE_CAPACITY);
}

static void audio_trace_append(struct AudioTraceRecord record)
{
    if (!audio_trace_records) return;
    if (audio_trace_count == KFX_AUDIO_TRACE_CAPACITY) {
        ++audio_trace_overflow;
        return;
    }
    audio_trace_records[audio_trace_count++] = record;
}
#endif
