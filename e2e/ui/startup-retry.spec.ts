import { expect, test } from "@playwright/test";

import { isTransientStartupFailure } from "./startup-retry";

test("only ERR_NO_BUFFER_SPACE makes a failed start worth retrying", () => {
  expect(isTransientStartupFailure(["net::ERR_NO_BUFFER_SPACE"])).toBe(true);
  expect(isTransientStartupFailure(["net::ERR_ABORTED", "net::ERR_NO_BUFFER_SPACE"])).toBe(true);
  expect(isTransientStartupFailure([])).toBe(false);
  expect(isTransientStartupFailure(["net::ERR_CONNECTION_REFUSED"])).toBe(false);
});
