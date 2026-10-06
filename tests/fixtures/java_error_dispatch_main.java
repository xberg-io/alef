package com.test;

import java.lang.foreign.Arena;

public final class ErrorDispatchMain {
    private ErrorDispatchMain() {}

    public static void main(String[] args) throws Throwable {
        try (Arena arena = Arena.ofConfined()) {
            NativeLib.context = arena.allocateFrom("native failure");
            expect(1, ConversionErrorException.class);
            expect(2, CoreErrorException.class);
            expect(3, PanicException.class);
            expect(5, OperationCancelledException.class);
            expect(TYPED_CODE, RejectedException.class);
        }
    }

    private static void expect(int code, Class<? extends Throwable> expected) throws Throwable {
        NativeLib.code = code;
        try {
            ErrorDispatchProbe.runCheck();
            throw new AssertionError("error code did not throw: " + code);
        } catch (Throwable failure) {
            if (!expected.isInstance(failure)) {
                throw new AssertionError("wrong exception for code " + code + ": " + failure, failure);
            }
        }
    }
}
