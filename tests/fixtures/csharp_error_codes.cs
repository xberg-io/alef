using System;
using System.Runtime.InteropServices;
using Test;

internal static class Program
{
    private static void Main()
    {
        AssertError(2, "invalid input: unannotated", typeof(InvalidInputException));
        AssertError(109, "invalid input: explicit code", typeof(InvalidInputException));
        AssertError(731, "invalid input: unknown code", typeof(InvalidInputException));
        AssertError(2, "unclassified native error", typeof(RequestErrorException));
        AssertError(731, "unclassified native error", typeof(TestException));
        var inner = new Exception("inner");
        var legacy = new InvalidInputException("legacy", inner);
        if (legacy.Code != 0 || legacy.Message != "legacy" || legacy.InnerException != inner)
            throw new Exception("Legacy constructor changed");
        if (new InvalidInputException("legacy").Code != 0)
            throw new Exception("Legacy message constructor changed");
    }

    private static void AssertError(int code, string message, Type type)
    {
        NativeMethods.Code = code;
        NativeMethods.Context = Marshal.StringToCoTaskMemUTF8(message);
        try
        {
            var error = (TestException)TestException.FromLastError("fallback");
            if (error.GetType() != type || error.Code != code || error.Message != message)
                throw new Exception($"Expected {type.Name}/{code}/{message}; got {error.GetType().Name}/{error.Code}/{error.Message}");
        }
        finally
        {
            Marshal.FreeCoTaskMem(NativeMethods.Context);
        }
    }
}

namespace Test
{
    internal static class NativeMethods
    {
        internal static int Code;
        internal static IntPtr Context;
        internal static int LastErrorCode() => Code;
        internal static IntPtr LastErrorContext() => Context;
    }
}
