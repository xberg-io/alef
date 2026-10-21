#include <assert.h>
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
typedef uint64_t SAMPLEAlefHandle;
static int code;
static int chunks;
static int chunk_frees;
static int stream_frees;
static int client_frees;
static const char* message;
static SAMPLEAlefHandle sample_client_new(const char* key, const char* url, uint64_t timeout, uint32_t retry, void* extra) {
    (void)key; (void)url; (void)timeout; (void)retry; (void)extra; return 1;
}
static SAMPLEAlefHandle sample_client_chat_stream_start(SAMPLEAlefHandle client, SAMPLEAlefHandle request) {
    (void)client; (void)request;
    if (strcmp(getenv("STREAM_MODE"), "immediate") == 0) { code = 1; message = "limited"; return 0; }
    code = 0; return 2;
}
static SAMPLEAlefHandle sample_client_chat_stream_next(SAMPLEAlefHandle stream) {
    (void)stream;
    if (chunks++ < 2) { code = 0; return 3; }
    if (strcmp(getenv("STREAM_MODE"), "clean") != 0) { code = 1; message = strcmp(getenv("STREAM_MODE"), "wrong") == 0 ? "different" : "limited"; }
    return 0;
}
static int sample_last_error_code(void) { return code; }
static const char* sample_last_error_context(void) { return message; }
static void sample_stream_chunk_free(SAMPLEAlefHandle chunk) { (void)chunk; chunk_frees++; code = 0; }
static void sample_client_chat_stream_free(SAMPLEAlefHandle stream) { (void)stream; stream_frees++; code = 0; message = NULL; }
static void sample_client_free(SAMPLEAlefHandle client) { (void)client; client_frees++; code = 0; message = NULL; }
GENERATED_BODY
int main(void) {
    test_late_stream_failure();
    int immediate = strcmp(getenv("STREAM_MODE"), "immediate") == 0;
    assert(chunk_frees == (immediate ? 0 : 2));
    assert(stream_frees == (immediate ? 0 : 1));
    assert(client_frees == 1);
}
