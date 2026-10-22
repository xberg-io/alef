package com.test;

import java.lang.foreign.Arena;
import java.lang.foreign.MemorySegment;

/** Scripted stand-in for the native library: each `next` call plays the next event. */
final class NativeLib {
    interface Invoker {
        Object invoke(Object... args);
    }

    static String[] events = new String[0];
    static int position;
    static int streamFreeCalls;
    static int lastCode;
    static String lastMessage;

    private NativeLib() {}

    static void script(String... scripted) {
        events = scripted;
        position = 0;
        streamFreeCalls = 0;
        lastCode = 0;
        lastMessage = null;
    }

    static TestLibRsException lastErrorException(int code, String message) {
        if (code == 3) return new ServerErrorException(message, 500);
        return new TestLibRsException(code, message);
    }

    static final Invoker TL_CHAT_COMPLETION_REQUEST_FROM_JSON = args -> MemorySegment.ofAddress(7);
    static final Invoker TL_CHAT_COMPLETION_REQUEST_FREE = args -> null;
    static final Invoker TL_DEFAULT_CLIENT_CHAT_STREAM_START = args -> {
        if (events.length > 0 && events[0].startsWith("START_ERR:")) {
            lastCode = Integer.parseInt(events[0].split(":", 3)[1]);
            lastMessage = events[0].split(":", 3)[2];
            return MemorySegment.NULL;
        }
        return MemorySegment.ofAddress(5);
    };
    static final Invoker TL_DEFAULT_CLIENT_CHAT_STREAM_FREE = args -> {
        streamFreeCalls++;
        return null;
    };
    static final Invoker TL_DEFAULT_CLIENT_CHAT_STREAM_NEXT = args -> {
        lastCode = 0;
        lastMessage = null;
        String event = events[position++];
        if (event.startsWith("ERR:")) {
            lastCode = Integer.parseInt(event.split(":", 3)[1]);
            lastMessage = event.split(":", 3)[2];
            return MemorySegment.NULL;
        }
        if (event.equals("END")) return MemorySegment.NULL;
        return MemorySegment.ofAddress(1000 + position);
    };
    static final Invoker TL_CHAT_COMPLETION_CHUNK_TO_JSON = args -> {
        long address = ((MemorySegment) args[0]).address();
        return Arena.global().allocateFrom(events[(int) (address - 1000) - 1]);
    };
    static final Invoker TL_CHAT_COMPLETION_CHUNK_FREE = args -> null;
    static final Invoker TL_FREE_STRING = args -> null;
    static final Invoker TL_LAST_ERROR_CODE = args -> (long) lastCode;
    static final Invoker TL_LAST_ERROR_CONTEXT = args ->
        lastMessage == null ? MemorySegment.NULL : Arena.global().allocateFrom(lastMessage);
}
