package com.test;

final class NativeLib {
    static int freeCalls;
    static final Invoker TEST_RESOURCE_FREE = new Invoker(true);
    static final Invoker TEST_RESOURCE_CONSUME = new Invoker(false);
    static final Invoker TEST_LAST_ERROR_CODE = new Invoker(false);
    static final Invoker TEST_LAST_ERROR_CONTEXT = new Invoker(false);

    private NativeLib() {}

    static TestLibRsException lastErrorException(int code, String message) {
        return new TestLibRsException(code, message);
    }

    static final class Invoker {
        private final boolean countsFree;

        Invoker(boolean countsFree) {
            this.countsFree = countsFree;
        }

        Object invoke(Object... ignored) {
            if (countsFree) freeCalls++;
            return null;
        }
    }
}
