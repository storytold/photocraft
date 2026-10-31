import { beforeAll, afterAll, expect } from "vitest";
import { generatorDescribe, generatorTest, getActiveBridge } from "@tencent/ps-generator-bridge-generator/vitest";
import { selectedCases } from "../src/cases.js";
import { LiveHarness } from "../src/harness.js";

const cases = selectedCases();

generatorDescribe("PhotoCraft hue/saturation live RGB/8 parity", () => {
  let harness;
  beforeAll(async () => {
    harness = new LiveHarness(getActiveBridge());
    await harness.start();
  }, 120000);
  afterAll(async () => { await harness?.finish(); }, 60000);

  for (const c of cases) generatorTest(c.id, async () => {
    const result = await harness.execute(c);
    expect(result.status).toBe("PASS");
  }, 330000);
});
