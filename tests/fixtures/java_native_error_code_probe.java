package fixture;

public class CodeProbe {
  public static void main(String[] args) {
    for (int code : new int[] {100, 101, 2, 5, 987}) {
      FixtureRsException error = NativeLib.lastErrorException(code, "native context");
      if (error.getCode() != code) throw new AssertionError("code " + code + " became " + error.getCode());
      if (!error.getMessage().equals("native context")) throw new AssertionError("message lost");
      if (code == 100 || code == 101) {
        if (!(error instanceof TooManyRedirectsException)) throw new AssertionError("specific type lost");
        if (!(error instanceof PolicyErrorException)) throw new AssertionError("legacy base lost");
        if (!(error instanceof RequestErrorException)) throw new AssertionError("shared base lost");
      }
      if (code == 2 && !(error instanceof CoreErrorException)) throw new AssertionError("infrastructure type lost");
      if (code == 5 && !(error instanceof OperationCancelledException)) throw new AssertionError("cancelled type lost");
      if (code == 987 && error.getClass() != FixtureRsException.class) throw new AssertionError("fallback type changed");
    }
    Throwable cause = new IllegalArgumentException("cause");
    if (new TooManyRedirectsException("legacy").getCode() != -1) throw new AssertionError("legacy constructor changed");
    if (new TooManyRedirectsException("legacy", cause).getCause() != cause) throw new AssertionError("cause lost");
    if (new FixtureRsException(42, "legacy").getCode() != 42) throw new AssertionError("code constructor changed");
    System.out.println("CODE_CASES=5 LEGACY_CONSTRUCTORS=3");
  }
}
