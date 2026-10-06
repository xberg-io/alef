package com.test;

import java.lang.foreign.MemorySegment;

final class NativeLib {
    static int code;
    static MemorySegment context = MemorySegment.NULL;
    static final Invoker TEST_LAST_ERROR_CODE = new Invoker(true);
    static final Invoker TEST_LAST_ERROR_CONTEXT = new Invoker(false);

    private NativeLib() {}

LAST_ERROR_EXCEPTION

    static final class Invoker {
        private final boolean returnsCode;

        Invoker(boolean returnsCode) {
            this.returnsCode = returnsCode;
        }

        Object invoke(Object... ignored) {
            return returnsCode ? (long) code : context;
        }
    }
}
