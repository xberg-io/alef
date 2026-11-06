import Foundation
import SampleCore
import RustBridge

func runReusable() {
  let input = SampleCore.makeOwnedInput(value: 7)
  precondition(SampleCore.consumeOwnedInputs(inputs: [input, input]) == 14)
  precondition(RustBridge.ownedInputIsOwnedForProbe(input))
  precondition(SampleCore.consumeOwnedInputs(inputs: [input, input]) == 14)
  withExtendedLifetime(input) {
    precondition(SampleCore.ownedInputDrops() == 4)
  }
}
func runAsyncReusable() async throws {
  let input = SampleCore.makeOwnedInput(value: 7)
  let first = try await SampleCore.consumeOwnedInputsAsync(inputs: [input, input])
  precondition(first == 14)
  let second = try await SampleCore.consumeOwnedInputsAsync(inputs: [input, input])
  precondition(second == 14)
  precondition(RustBridge.ownedInputIsOwnedForProbe(input))
  withExtendedLifetime(input) {
    precondition(SampleCore.ownedInputDrops() == 4)
  }
}
let mode = ProcessInfo.processInfo.environment["ALEF_OWNED_VALUES_CASE"] ?? "unique"
FileHandle.standardError.write(Data("owned_vector_case=\(mode)\n".utf8))
if mode == "unique" {
  let first = SampleCore.makeOwnedInput(value: 7)
  let second = SampleCore.makeOwnedInput(value: 11)
  precondition(SampleCore.consumeOwnedInputs(inputs: [first, second]) == 18)
} else if mode == "clone-reuse" {
  runReusable()
  precondition(SampleCore.ownedInputDrops() == 5)
} else if mode == "clone-async-reuse" {
  try await runAsyncReusable()
} else if mode == "moving-unique" {
  let first = SampleCore.makeMovingInput(value: 5)
  let second = SampleCore.makeMovingInput(value: 13)
  precondition(SampleCore.consumeMovingInputs(inputs: [first, second]) == 18)
} else if mode == "moving-state" {
  let input = SampleCore.makeMovingInput(value: 7)
  precondition(RustBridge.movingInputIsOwnedForProbe(input))
  precondition(SampleCore.consumeMovingInputs(inputs: [input]) == 7)
  precondition(!RustBridge.movingInputIsOwnedForProbe(input))
} else {
  fatalError("unknown fixture case")
}
print("owned_vector_success=\(mode)")
