import { describe, expect, it } from "vitest";
import type { Ledger } from "../api/types";
import { vaultHealth } from "./vaultHealth";

const base: Ledger = {
  vault_balance: 30_000_000,
  cache: 20_000_000,
  unpaid_obligations: 2_000_000,
  reserved_fees: 10_000,
  unclaimable: 0,
  operator_float: 7_990_000,
  shortfall: 0,
  last_reconciliation_height: 1_000,
};

describe("vault health", () => {
  it("is working when the vault covers the Cache and what is owed", () => {
    const v = vaultHealth(base, 1_002);
    expect(v.health).toBe("ok");
    expect(v.detail).toContain("covers the Deep Cache (0.2 ZEC)");
    expect(v.detail).toContain("0.0799 ZEC to spare");
    expect(v.detail).toContain("Checked at block 1,000 (2 blocks ago)");
  });

  it("warns with the amount when the vault is short", () => {
    const v = vaultHealth(
      {
        ...base,
        vault_balance: 17_627_750,
        cache: 20_032_750,
        unpaid_obligations: 0,
        reserved_fees: 0,
        operator_float: 0,
        shortfall: 2_405_000,
      },
      1_000,
    );
    expect(v.health).toBe("warn");
    expect(v.detail).toContain("Short by 0.024 ZEC");
    expect(v.detail).toContain("holds 0.1762 ZEC for 0.2003 ZEC owed");
  });

  it("warns when the balance was read long before the tip", () => {
    const v = vaultHealth(base, 1_000 + 3_634);
    expect(v.health).toBe("warn");
    expect(v.detail).toContain("read 3634 blocks ago");
  });
});
