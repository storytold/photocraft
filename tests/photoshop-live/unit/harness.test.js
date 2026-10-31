import { describe, expect, it } from "vitest";
import { LiveHarness, requiredArchivedReference } from "../src/harness.js";

const path = "C:/photocraft-tests/unique-case/input.psd";

/** Build a Bridge fake that records read-only reconciliation and close calls. */
function harnessWithClose(reply) {
  const calls = [];
  const harness = new LiveHarness({});
  harness.registerOwned(11, path);
  harness.connection = {
    /** Route only the two wire methods needed by owned-document close tests. */
    async invoke(method, args) {
      calls.push({ method, args });
      if (method === "document/list") return reply.list();
      if (method === "document/close") return reply.close();
      throw new Error(`Unexpected mock bridge method: ${method}`);
    },
    /** Match RawConnection's local shutdown without touching Photoshop. */
    close() {},
  };
  return { harness, calls };
}

const ownedDocument = { id: 11, filePath: path };

describe("Photoshop harness safety", () => {
  it("requires the archived issue export before comparing live pixels", () => {
    expect(() => requiredArchivedReference("C:/nonexistent-photocraft-issue-oracle"))
      .toThrow(/SETUP_UNVERIFIED: required archived Photoshop export missing/);
  });

  it("does not replay an uncertain close when the document vanished", async () => {
    let lists = 0;
    const { harness, calls } = harnessWithClose({
      list: () => ++lists === 1 ? [ownedDocument] : [],
      close: () => { throw new Error("lost response"); },
    });
    await expect(harness.closeOwned(11)).rejects.toThrow(/document is absent/);
    expect(harness.owned.has(11)).toBe(false);
    await harness.finish();
    expect(calls.filter(c => c.method === "document/close")).toHaveLength(1);
  });

  it("reports an ambiguous still-open close without automatically retrying", async () => {
    const { harness, calls } = harnessWithClose({
      list: () => [ownedDocument],
      close: () => { throw new Error("lost response"); },
    });
    await expect(harness.closeOwned(11)).rejects.toThrow(/automatic retry is unsafe/);
    await expect(harness.finish()).rejects.toThrow(/automatic retry is unsafe/);
    expect(calls.filter(c => c.method === "document/close")).toHaveLength(1);
  });

  it("does not retry when the close response and follow-up list both fail", async () => {
    let lists = 0;
    const { harness, calls } = harnessWithClose({
      list: () => {
        if (++lists === 1) return [ownedDocument];
        throw new Error("list unavailable");
      },
      close: () => { throw new Error("lost response"); },
    });
    await expect(harness.closeOwned(11)).rejects.toThrow(/document state is unknown/);
    await expect(harness.finish()).rejects.toThrow(/document state is unknown/);
    expect(calls.filter(c => c.method === "document/close")).toHaveLength(1);
  });

  it("refuses to close a reused ID with a different document path", async () => {
    const { harness, calls } = harnessWithClose({
      list: () => [{ id: 11, filePath: "C:/other/user-document.psd" }],
      close: () => ({ id: 11 }),
    });
    await expect(harness.closeOwned(11)).rejects.toThrow(/current file path is not test-owned/);
    expect(calls.filter(c => c.method === "document/close")).toHaveLength(0);
  });

  it("closes a document once when its ID and associated test path match", async () => {
    const { harness, calls } = harnessWithClose({
      list: () => [ownedDocument],
      close: () => ({ id: 11 }),
    });
    await harness.closeOwned(11);
    await harness.finish();
    expect(calls.filter(c => c.method === "document/close")).toHaveLength(1);
    expect(harness.owned.has(11)).toBe(false);
  });

  it("cleans up once when saveAs changes the owned path but its response is lost", async () => {
    const newPath = "C:/photocraft-tests/unique-case/baseline.psd";
    let livePath = path;
    const calls = [];
    const harness = new LiveHarness({});
    harness.registerOwned(11, path);
    harness.connection = {
      /** Simulate Photoshop saving under the new path before losing the reply. */
      async invoke(method, args) {
        calls.push({ method, args });
        if (method === "document/list") return [{ id: 11, filePath: livePath }];
        if (method === "document/saveAs") {
          livePath = args.filePath;
          throw new Error("save response lost");
        }
        if (method === "document/close") return { id: 11 };
        throw new Error(`Unexpected mock bridge method: ${method}`);
      },
      /** Match RawConnection's local shutdown without touching Photoshop. */
      close() {},
    };
    await expect(harness.saveOwnedAs(11, newPath)).rejects.toThrow(/save response lost/);
    expect(livePath).toBe(newPath);
    await harness.closeOwned(11);
    await harness.finish();
    expect(calls.filter(c => c.method === "document/saveAs")).toHaveLength(1);
    expect(calls.filter(c => c.method === "document/close")).toHaveLength(1);
    expect(harness.owned.has(11)).toBe(false);
  });
});
