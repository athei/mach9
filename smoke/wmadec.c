/* The WMA program bundle-wine.sh builds for each x86 PE architecture and
 * runs under the bundled wine. It decodes WMA as a game does through the WMA
 * decoder DMO (CLSID_CWMADecMediaObject, wmadmod.dll's class, which Wine
 * implements in winegstreamer): it creates the DMO, sets a WMAv2 input type
 * and a 16-bit PCM output type, checks the stream info and input status
 * calls, then feeds the packets below one at a time, draining the output
 * after each, and requires PCM samples that are not all zero. That goes
 * through winegstreamer.so to the bundled GStreamer and gst-libav's
 * avdec_wmav2. It prints one line per step that fails and exits with the
 * step's number (the enum below).
 *
 * The packets are 0.25 s of a 440 Hz sine, stereo at 44100 Hz, encoded to
 * WMAv2 at 64 kbit/s, which gives 6 packets of 371 bytes (the block
 * alignment) and the 10 bytes of extra format data below. They were made
 * with an ffmpeg built from GStreamer's FFmpeg port at the commit redist.env
 * pins, configured with
 *
 *   configure --disable-everything --disable-autodetect --disable-network
 *       --enable-protocol=file --enable-demuxer=pcm_s16le
 *       --enable-decoder=pcm_s16le --enable-encoder=wmav2 --enable-muxer=wav
 *       --enable-filter=aresample,aformat,anull --enable-swresample
 *
 * from a raw sine (perl -e 'for $i (0..11024) { $v = int(12000 * sin(2 *
 * 3.14159265 * 440 * $i / 44100)); print pack("ss", $v, $v) }' > sine.raw)
 * with
 *
 *   ffmpeg -f s16le -ar 44100 -ac 2 -i sine.raw -c:a wmav2 -b:a 64k -f wav wma.wav
 *
 * and taken from the WAV file's data chunk, split at the block alignment.
 * The encoder fills the end of each packet with 0x4e bytes; they are left
 * out here and put back when a packet is fed. */

#define COBJMACROS
#include <stdio.h>
#include <string.h>
#include <windows.h>
#include <mmreg.h>
#include <mediaobj.h>

#if defined(__x86_64__)
#define ARCH "x86_64"
#elif defined(__i386__)
#define ARCH "i686"
#else
#error "unknown architecture"
#endif

/* Defined here, so the program links nothing but ole32. */
static const GUID iid_unknown = { 0x00000000, 0x0000, 0x0000, { 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46 } };
static const GUID clsid_wmadec = { 0x2eeb4adf, 0x4578, 0x4d10, { 0xbc, 0xa7, 0xbb, 0x95, 0x5f, 0x56, 0x32, 0x0a } };
static const GUID iid_media_object = { 0xd8ad0f58, 0x5494, 0x4102, { 0x97, 0xc5, 0xec, 0x79, 0x8e, 0x59, 0xbc, 0xf4 } };
static const GUID iid_media_buffer = { 0x59eff8b9, 0x938c, 0x4a26, { 0x82, 0xf2, 0x95, 0xcb, 0x84, 0xcd, 0xc8, 0x37 } };
static const GUID mediatype_audio = { 0x73647561, 0x0000, 0x0010, { 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71 } };
static const GUID subtype_wmaudio2 = { 0x00000161, 0x0000, 0x0010, { 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71 } };
static const GUID subtype_pcm = { 0x00000001, 0x0000, 0x0010, { 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71 } };
static const GUID format_waveformatex = { 0x05589f81, 0xc356, 0x11ce, { 0xbf, 0x01, 0x00, 0xaa, 0x00, 0x55, 0x59, 0x5a } };

#define ARRAY_SIZE(a) (sizeof(a) / sizeof((a)[0]))
#define RATE 44100
#define CHANNELS 2
#define BLOCK_ALIGN 371
#define FILL 0x4e

static const unsigned char packet0[] =
{
    0xf4, 0xbf, 0xd0, 0x00, 0x00, 0x03, 0xfd, 0x00, 0x00, 0x00, 0x3f, 0x3f,
    0xcb, 0xf2, 0xfc, 0xff, 0x3f, 0xcb, 0xf2, 0xfc, 0xff, 0x3f, 0xcb, 0xf2,
    0xfc, 0xf0, 0x1e, 0x02, 0xc0, 0x58, 0x0f, 0x01, 0xf9, 0xaf, 0x35, 0xe6,
    0xeb, 0x1e, 0xb1, 0xab, 0x82, 0xb8, 0x73, 0xdd, 0xd2, 0xce, 0x96, 0xc1,
    0x2f, 0x84, 0x6d, 0x8e, 0xc1, 0xb1, 0xd3, 0xff, 0xf3, 0xee, 0x91, 0x9f,
    0x8b, 0x9d, 0x3e, 0x7c, 0x11, 0xed, 0x71, 0x15, 0xe7, 0xaf, 0x78, 0x0a,
    0xfc, 0x00, 0x7c, 0x07, 0xda, 0x00, 0x3e, 0x03, 0xeb, 0x00, 0x1f, 0x01,
    0x5e, 0x00, 0x0b, 0x88, 0x52, 0xae, 0x22, 0xbf, 0x93, 0x7f, 0x93, 0xff,
    0x9e, 0xc1, 0x30, 0xe2, 0x2d, 0xb1, 0xf9, 0xec, 0xe9, 0x6a, 0xc7, 0xae,
    0x1c, 0x05, 0xe6, 0xb2, 0x9f, 0xcf, 0xf5, 0x94, 0x5b, 0xff, 0x7c, 0x56,
    0xee, 0x3e, 0x35, 0xae, 0x25, 0xb5, 0xb7, 0xcb, 0x4f, 0xdf, 0xbe, 0x7d,
    0x4b, 0xfa, 0x1f, 0x53, 0x4d, 0x14, 0x53, 0x4a, 0x28, 0x4d, 0x28, 0xa1,
    0x29, 0x42, 0x12, 0x94, 0x21, 0x29, 0x42, 0x12, 0x90, 0x82, 0x90, 0x82,
    0x90, 0x09, 0x00, 0x90, 0x09, 0x00, 0x90, 0x09, 0x00, 0x90, 0x09, 0x00,
    0x90, 0x09, 0x00, 0x90, 0x09, 0x00, 0x90, 0x09, 0x00, 0x98, 0x98, 0x98,
    0x98, 0x98, 0x98, 0x98, 0x7a, 0xc0,
};
static const unsigned char packet1[] =
{
    0xf5, 0x7f, 0xd0, 0x00, 0x00, 0x03, 0xfd, 0x00, 0x00, 0x00, 0x3c, 0x69,
    0x81, 0x49, 0xb7, 0x2d, 0x3a, 0x7d, 0x7c, 0x05, 0xc8, 0x00, 0x1e, 0x03,
    0xf2, 0x00, 0x0c, 0x05, 0x80, 0xd2, 0xf8, 0x26, 0x25, 0xeb,
};
static const unsigned char packet2[] =
{
    0xf5, 0xbf, 0xd0, 0x00, 0x00, 0x03, 0xfd, 0x00, 0x00, 0x00, 0x3c, 0x78,
    0x9a, 0x8e, 0x82, 0x3c, 0xe3, 0xce, 0x23, 0x57, 0x80, 0xf5, 0x40, 0x05,
    0x18, 0x09, 0x63, 0x52, 0x61, 0xeb,
};
static const unsigned char packet3[] =
{
    0xf5, 0x7f, 0xd0, 0x00, 0x00, 0x03, 0xfd, 0x00, 0x00, 0x00, 0x3c, 0x69,
    0x00, 0xa5, 0x1c, 0x56, 0xfc, 0xaa, 0xcf, 0x80, 0xd2, 0x80, 0x07, 0xc0,
    0x7a, 0x00, 0x00, 0xe9, 0x7f, 0xce, 0x8a, 0x0a, 0x40, 0x97, 0xac,
};
static const unsigned char packet4[] =
{
    0xf5, 0xbf, 0xd0, 0x00, 0x00, 0x03, 0xfd, 0x00, 0x00, 0x00, 0x1c, 0x54,
    0x90, 0x8a, 0x5f, 0xfe, 0x58, 0x26, 0x3c, 0x07, 0xd0, 0x00, 0x1e, 0x03,
    0x1f, 0x00, 0x17, 0x10, 0xdc, 0x4f, 0xd2, 0x80, 0x48, 0x7a, 0xc0,
};
static const unsigned char packet5[] =
{
    0xf5, 0xbf, 0xd0, 0x00, 0x00, 0x03, 0xfd, 0x00, 0x00, 0x00, 0x2d, 0xdc,
    0x4f, 0x90, 0xb1, 0x5b, 0xb7, 0xdb, 0xed, 0xfc, 0x6f, 0xca, 0x1f, 0x5b,
    0xbf, 0x5f, 0xaf, 0xd7, 0x13, 0xe7, 0x66, 0xdf, 0xf9, 0xf9, 0xbf, 0x37,
    0xf9, 0xdb, 0xdf, 0xa3, 0x28, 0xcf, 0x67, 0x11, 0x1c, 0x43, 0x56, 0x07,
    0x56, 0x07, 0x3a, 0x5a, 0x8c, 0xab, 0x8f, 0x01, 0xfe, 0x80, 0x0f, 0x80,
    0xb8, 0xc0, 0x03, 0xc0, 0x49, 0x80, 0x01, 0xe0, 0xd5, 0x3f, 0x11, 0xbc,
    0xd5, 0x09, 0xfd, 0xd6, 0x3d, 0x63, 0xe0, 0x3f, 0xcf, 0xf6, 0xfe, 0xa3,
    0xe5, 0xae, 0x2e, 0x25, 0xaa, 0x11, 0x56, 0x97, 0xef, 0xdf, 0xd2, 0x98,
    0x42, 0x28, 0xa1, 0x08, 0x12, 0x52, 0x94, 0xa4, 0xb0, 0x21, 0x00, 0x06,
    0x92, 0x49, 0x60, 0x00, 0x0d, 0x92, 0x49, 0xd0, 0x00, 0x2a, 0x49, 0xb8,
    0x01, 0x79, 0x7a, 0xc0,
};

static const struct { const unsigned char *data; size_t size; } packets[] =
{
    { packet0, sizeof(packet0) }, { packet1, sizeof(packet1) }, { packet2, sizeof(packet2) },
    { packet3, sizeof(packet3) }, { packet4, sizeof(packet4) }, { packet5, sizeof(packet5) },
};

/* The WAVEFORMATEX of WAVE_FORMAT_WMAUDIO2 with its extra data, as the
 * encoder wrote it: no samples per block given, the exponent VLC flag set. */
#pragma pack(push, 1)
struct wma_format
{
    WAVEFORMATEX wfx;
    BYTE extra[10];
};
#pragma pack(pop)

enum
{
    FAIL_CREATE = 1,
    FAIL_INPUT_TYPE,
    FAIL_OUTPUT_TYPE,
    FAIL_STREAM_INFO,
    FAIL_INPUT_STATUS,
    FAIL_PROCESS_INPUT,
    FAIL_PROCESS_OUTPUT,
    FAIL_NO_OUTPUT,
};

/* A minimal IMediaBuffer over memory of its own. The DMO may hold on to an
 * input buffer until it has decoded it, so each one lives as long as its
 * references. */
struct buffer
{
    IMediaBuffer IMediaBuffer_iface;
    LONG refcount;
    DWORD length, max_length;
    BYTE data[];
};

static struct buffer *impl_from_IMediaBuffer(IMediaBuffer *iface)
{
    return CONTAINING_RECORD(iface, struct buffer, IMediaBuffer_iface);
}

static HRESULT WINAPI buffer_QueryInterface(IMediaBuffer *iface, REFIID iid, void **out)
{
    if (IsEqualGUID(iid, &iid_unknown) || IsEqualGUID(iid, &iid_media_buffer))
    {
        IMediaBuffer_AddRef(iface);
        *out = iface;
        return S_OK;
    }
    *out = NULL;
    return E_NOINTERFACE;
}

static ULONG WINAPI buffer_AddRef(IMediaBuffer *iface)
{
    return InterlockedIncrement(&impl_from_IMediaBuffer(iface)->refcount);
}

static ULONG WINAPI buffer_Release(IMediaBuffer *iface)
{
    struct buffer *buffer = impl_from_IMediaBuffer(iface);
    ULONG refcount = InterlockedDecrement(&buffer->refcount);

    if (!refcount) HeapFree(GetProcessHeap(), 0, buffer);
    return refcount;
}

static HRESULT WINAPI buffer_SetLength(IMediaBuffer *iface, DWORD length)
{
    struct buffer *buffer = impl_from_IMediaBuffer(iface);

    if (length > buffer->max_length) return E_INVALIDARG;
    buffer->length = length;
    return S_OK;
}

static HRESULT WINAPI buffer_GetMaxLength(IMediaBuffer *iface, DWORD *length)
{
    *length = impl_from_IMediaBuffer(iface)->max_length;
    return S_OK;
}

static HRESULT WINAPI buffer_GetBufferAndLength(IMediaBuffer *iface, BYTE **data, DWORD *length)
{
    struct buffer *buffer = impl_from_IMediaBuffer(iface);

    if (data) *data = buffer->data;
    if (length) *length = buffer->length;
    return S_OK;
}

static IMediaBufferVtbl buffer_vtbl =
{
    buffer_QueryInterface, buffer_AddRef, buffer_Release,
    buffer_SetLength, buffer_GetMaxLength, buffer_GetBufferAndLength,
};

static struct buffer *create_buffer(DWORD max_length)
{
    struct buffer *buffer = HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY,
                                      sizeof(*buffer) + max_length);

    buffer->IMediaBuffer_iface.lpVtbl = &buffer_vtbl;
    buffer->refcount = 1;
    buffer->max_length = max_length;
    return buffer;
}

int main(void)
{
    struct wma_format in_format = { { WAVE_FORMAT_WMAUDIO2, CHANNELS, RATE, 8000, BLOCK_ALIGN, 16, 10 },
                                    { 0, 0, 0, 0, 1, 0, 0, 0, 0, 0 } };
    WAVEFORMATEX out_format = { WAVE_FORMAT_PCM, CHANNELS, RATE, RATE * CHANNELS * 2, CHANNELS * 2, 16, 0 };
    DMO_MEDIA_TYPE in_type = { 0 }, out_type = { 0 };
    DMO_OUTPUT_DATA_BUFFER output;
    struct buffer *in, *out;
    DWORD flags, status, pcm_bytes = 0, i, j, k, rounds;
    IMediaObject *dmo;
    int nonzero = 0;
    HRESULT hr;

    CoInitializeEx(NULL, COINIT_MULTITHREADED);
    if (FAILED(hr = CoCreateInstance(&clsid_wmadec, NULL, CLSCTX_INPROC_SERVER, &iid_media_object,
                                     (void **)&dmo)))
    {
        printf("wine-build smoke test: %s cannot create the WMA decoder DMO, 0x%08lx\n", ARCH, hr);
        return FAIL_CREATE;
    }

    in_type.majortype = mediatype_audio;
    in_type.subtype = subtype_wmaudio2;
    in_type.bTemporalCompression = TRUE;
    in_type.formattype = format_waveformatex;
    in_type.cbFormat = sizeof(in_format);
    in_type.pbFormat = (BYTE *)&in_format;
    if ((hr = IMediaObject_SetInputType(dmo, 0, &in_type, 0)) != S_OK)
    {
        printf("wine-build smoke test: %s SetInputType (WMAv2) failed, 0x%08lx\n", ARCH, hr);
        return FAIL_INPUT_TYPE;
    }
    out_type.majortype = mediatype_audio;
    out_type.subtype = subtype_pcm;
    out_type.bFixedSizeSamples = TRUE;
    out_type.lSampleSize = out_format.nBlockAlign;
    out_type.formattype = format_waveformatex;
    out_type.cbFormat = sizeof(out_format);
    out_type.pbFormat = (BYTE *)&out_format;
    if ((hr = IMediaObject_SetOutputType(dmo, 0, &out_type, 0)) != S_OK)
    {
        printf("wine-build smoke test: %s SetOutputType (PCM) failed, 0x%08lx\n", ARCH, hr);
        return FAIL_OUTPUT_TYPE;
    }
    if ((hr = IMediaObject_GetInputStreamInfo(dmo, 0, &flags)) != S_OK)
    {
        printf("wine-build smoke test: %s GetInputStreamInfo failed, 0x%08lx\n", ARCH, hr);
        return FAIL_STREAM_INFO;
    }
    if ((hr = IMediaObject_GetOutputStreamInfo(dmo, 0, &flags)) != S_OK)
    {
        printf("wine-build smoke test: %s GetOutputStreamInfo failed, 0x%08lx\n", ARCH, hr);
        return FAIL_STREAM_INFO;
    }

    out = create_buffer(16384);
    for (i = 0; i < ARRAY_SIZE(packets); i++)
    {
        if ((hr = IMediaObject_GetInputStatus(dmo, 0, &flags)) != S_OK ||
            !(flags & DMO_INPUT_STATUSF_ACCEPT_DATA))
        {
            printf("wine-build smoke test: %s GetInputStatus before packet %lu: 0x%08lx, flags %#lx\n",
                   ARCH, i, hr, flags);
            return FAIL_INPUT_STATUS;
        }
        in = create_buffer(BLOCK_ALIGN);
        memset(in->data, FILL, BLOCK_ALIGN);
        memcpy(in->data, packets[i].data, packets[i].size);
        in->length = BLOCK_ALIGN;
        hr = IMediaObject_ProcessInput(dmo, 0, &in->IMediaBuffer_iface, 0, 0, 0);
        IMediaBuffer_Release(&in->IMediaBuffer_iface);
        if (hr != S_OK)
        {
            printf("wine-build smoke test: %s ProcessInput of packet %lu failed, 0x%08lx\n", ARCH, i, hr);
            return FAIL_PROCESS_INPUT;
        }
        /* Drained until the DMO says it needs more input (S_FALSE); it marks
         * every buffer it fills as incomplete. */
        for (rounds = 0; rounds < 64; rounds++)
        {
            memset(&output, 0, sizeof(output));
            out->length = 0;
            output.pBuffer = &out->IMediaBuffer_iface;
            hr = IMediaObject_ProcessOutput(dmo, 0, 1, &output, &status);
            if (hr == S_FALSE) break;
            if (hr != S_OK)
            {
                printf("wine-build smoke test: %s ProcessOutput after packet %lu failed, 0x%08lx\n",
                       ARCH, i, hr);
                return FAIL_PROCESS_OUTPUT;
            }
            pcm_bytes += out->length;
            for (j = 0; j + 1 < out->length && !nonzero; j += 2)
                nonzero = out->data[j] || out->data[j + 1];
        }
        if (rounds == 64)
        {
            printf("wine-build smoke test: %s ProcessOutput after packet %lu never asked for more input\n",
                   ARCH, i);
            return FAIL_PROCESS_OUTPUT;
        }
    }
    k = pcm_bytes / out_format.nBlockAlign;
    if (!pcm_bytes || !nonzero)
    {
        printf("wine-build smoke test: %s the WMA decoder DMO gave %lu PCM bytes, %s\n", ARCH,
               pcm_bytes, nonzero ? "not all zero" : "all zero");
        return FAIL_NO_OUTPUT;
    }
    printf("wine-build smoke test: %s wmadec decoded %lu WMAv2 packets to %lu PCM frames\n", ARCH,
           (unsigned long)ARRAY_SIZE(packets), k);
    IMediaObject_Release(dmo);
    IMediaBuffer_Release(&out->IMediaBuffer_iface);
    return 0;
}
