package com.test;

import java.lang.foreign.Arena;
import java.lang.foreign.MemorySegment;

/** Holds the generated `chatStream` method with the members it needs from its generated class. */
final class StreamHarness {
    static final class Mapper {
        String writeValueAsString(Object value) { return "{}"; }

        <T> T readValue(String json, Class<T> type) throws Exception {
            return type.cast(new ChatCompletionChunk(json));
        }
    }

    static final Mapper STREAM_MAPPER = new Mapper();

    record HandleLease(MemorySegment handle) implements AutoCloseable {
        @Override
        public void close() {}
    }

    HandleLease borrowHandle() { return new HandleLease(MemorySegment.ofAddress(1)); }

    private static void checkLastFfiError() throws TestLibRsException {
        int code = (int) (long) NativeLib.TL_LAST_ERROR_CODE.invoke();
        if (code == 0) return;
        MemorySegment context = (MemorySegment) NativeLib.TL_LAST_ERROR_CONTEXT.invoke();
        throw NativeLib.lastErrorException(code, context.reinterpret(Long.MAX_VALUE).getString(0));
    }

GENERATED_METHOD
}
