package com.test;

final class BoolNarrowingMain {
    private static final long DIRTY_UPPER_BITS = 1L << Integer.SIZE;

    private static boolean decode(long value) {
        GENERATED_INVOCATION
        return primitiveResult != 0;
    }

    public static void main(String[] args) {
        long[] values = {0L, 1L, DIRTY_UPPER_BITS, DIRTY_UPPER_BITS | 1L};
        boolean[] expected = {false, true, false, true};
        for (int index = 0; index < values.length; index++) {
            if (decode(values[index]) != expected[index]) {
                throw new AssertionError("wrong bool result for widened carrier " + values[index]);
            }
        }
    }
}
