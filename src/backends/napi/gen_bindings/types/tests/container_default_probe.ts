import { roundTrip, requiredRecord } from "./index";

roundTrip({});
roundTrip({ maxDepth: 1 });
roundTrip({ browser: { mode: "always" } });
roundTrip({ browser: { timeout: 2500 } });
requiredRecord({ content: "kept", row: 3 });
// @ts-expect-error Required result DTO fields must stay required. ~keep
requiredRecord({});
