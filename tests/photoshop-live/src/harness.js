import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFile as execFileCallback } from "node:child_process";
import { copyFile, mkdir, readFile, stat, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";
import { RawConnection } from "@tencent/ps-generator-bridge-sdk";
import { WebSocket } from "ws";
import { getGeneratorConnectionOptions } from "@tencent/ps-generator-bridge-generator/vitest";
import { compareRgba } from "./compare.js";
import { rasterFor } from "./cases.js";
import { adjustmentScript, inspectIssueScript, prepareIssueBaseScript, prepareSourceScript } from "./photoshop-jsx.js";
import { assertPhotoshopImpact } from "./reference.js";

const execFile = promisify(execFileCallback);
const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(packageRoot, "..", "..");
const sourceFiles = [
  "crates/compose/src/adjust.rs", "crates/gpu/src/plan.rs", "crates/gpu/src/compose.wgsl",
  "crates/gpu/tests/it/parity.rs",
];
const testSuiteFiles = [
  "package.json", "pnpm-lock.yaml", "pnpm-workspace.yaml", "README.md", "live/parity.test.ts",
  "src/cases.js", "src/compare.js", "src/harness.js", "src/photoshop-jsx.js",
  "src/reference.js", "unit/compare.test.js",
  "unit/reference.test.js", "unit/harness.test.js",
];
const issueRelative = join("evid", "cases", "image-1-hs");
const runId = new Date().toISOString().replace(/[:.]/g, "-") + "-" + process.pid;
const artifactsRoot = resolve(process.env.PHOTOCRAFT_HS_ARTIFACTS ?? join(packageRoot, "artifacts", runId));
let sharedSharp;

/** Resolve Sharp through the pinned Bridge SDK so Photoshop and CLI PNGs use one decoder build. */
function loadSharedSharp() {
  const sdkManifest = join(packageRoot, "node_modules", "@tencent", "ps-generator-bridge-sdk", "package.json");
  return createRequire(sdkManifest)("@tencent/ps-generator-bridge-sdk/sharp").default;
}

/** Hash file bytes for fixture, source, binary, and test-suite provenance. */
async function sha256(path) {
  const hash = createHash("sha256");
  hash.update(await readFile(path));
  return hash.digest("hex");
}

/** Run a binary without a shell, with a two-minute timeout and bounded output. */
async function exec(binary, args, cwd = repoRoot) {
  const result = await execFile(binary, args, { cwd, timeout: 120000, maxBuffer: 16 * 1024 * 1024, windowsHide: true });
  return { stdout: result.stdout.trim(), stderr: result.stderr.trim() };
}

/** Record a checkout's commit, dirty state, tree, and changed-path source hashes. */
async function checkoutProvenance(root) {
  const [head, status, tree] = await Promise.all([
    exec("git", ["rev-parse", "HEAD"], root),
    exec("git", ["status", "--porcelain", "--untracked-files=normal"], root),
    exec("git", ["rev-parse", "HEAD^{tree}"], root),
  ]);
  const sources = {};
  for (const path of sourceFiles) if (existsSync(join(root, path))) sources[path] = await sha256(join(root, path));
  return { root,
    gitHead: head.stdout,
    gitTree: tree.stdout,
    gitDirty: status.stdout.length > 0,
    gitStatus: status.stdout.split(/\r?\n/).filter(Boolean),
    sourceSha256: sources };
}

/**
 * Bind the selected CLI hash to both source checkouts and every live-suite file.
 * Build-info is accepted only for a clean checkout whose binary/tree fields match.
 */
async function sourceProvenance(cli) {
  const engineRoot = resolve(process.env.PHOTOCRAFT_ENGINE_SOURCE ?? repoRoot);
  assert(existsSync(join(engineRoot, "Cargo.toml")), `Engine source has no Cargo.toml: ${engineRoot}`);
  const cliVersion = await exec(cli, ["--version"]);
  const cliSha256 = await sha256(cli);
  const [testCheckout, engineSource] = await Promise.all([
    checkoutProvenance(repoRoot), checkoutProvenance(engineRoot),
  ]);
  const testSuiteSha256 = {};
  for (const path of testSuiteFiles) testSuiteSha256[path] = await sha256(join(packageRoot, path));
  let sourceLink = { status: "BINARY_HASH_ONLY", reason: "No matching build-info manifest was supplied" };
  if (process.env.PHOTOCRAFT_ENGINE_BUILD_INFO) {
    const path = resolve(process.env.PHOTOCRAFT_ENGINE_BUILD_INFO);
    const info = JSON.parse(await readFile(path, "utf8"));
    assert.equal(info.executableSha256?.toLowerCase(), cliSha256, "Build-info CLI SHA-256 does not match selected binary");
    assert.equal(resolve(info.executablePath), cli, "Build-info executable path does not match selected CLI");
    assert.equal(info.commit, engineSource.gitHead, "Build-info commit does not match engine source checkout");
    assert.equal(resolve(info.sourceRoot), engineRoot, "Build-info source root does not match selected engine source");
    assert.equal(info.sourceTreeDigest, `git-tree:${engineSource.gitTree}`, "Build-info tree does not match engine source checkout");
    assert.equal(engineSource.gitDirty, false, "Build-info cannot verify a modified engine source checkout");
    sourceLink = { status: "VERIFIED_BUILD_INFO", buildInfoPath: path, buildInfoSha256: await sha256(path) };
  }
  return {
    testCheckout, engineSource, testSuiteSha256,
    cli: { path: cli, sha256: cliSha256, version: cliVersion.stdout || cliVersion.stderr, sourceLink },
    sdkVersion: JSON.parse(await readFile(join(packageRoot, "node_modules", "@tencent", "ps-generator-bridge-sdk", "package.json"), "utf8")).version,
    generatorVersion: JSON.parse(await readFile(join(packageRoot, "node_modules", "@tencent", "ps-generator-bridge-generator", "package.json"), "utf8")).version,
    sharpVersion: sharedSharp.versions.sharp,
  };
}

/** Normalize host snapshots so active-document and user-document checks are stable. */
function documentState(list, active) {
  return {
    activeId: active?.id ?? null,
    documents: list.map(d => ({ id: d.id, name: d.name, filePath: d.filePath ?? null, isDirty: d.isDirty })).sort((a, b) => a.id - b.id),
  };
}

/** Refuse ownership if an opened ID belonged to a user document or its copy vanished. */
function assertOwned(opened, before, expectedPath) {
  assert(Number.isSafeInteger(opened?.id) && opened.id > 0, "Host did not return a document ID");
  assert(!before.documents.some(d => d.id === opened.id), "Host reused a pre-existing user document ID");
  // A copied per-case path makes reopening an already-open user file impossible.
  assert(existsSync(expectedPath), "Owned source file vanished before document registration");
}

/** Require the archived Photoshop PNG before trusting an issue-fixture live export. */
export function requiredArchivedReference(issueRoot) {
  assert(issueRoot, "NOT_EXECUTED: set PHOTOCRAFT_HS_ISSUE_ROOT to the external issue folder");
  const archived = join(resolve(issueRoot), issueRelative, "ps-export.png");
  assert(existsSync(archived), `SETUP_UNVERIFIED: required archived Photoshop export missing: ${archived}`);
  return archived;
}

/** Compare Windows document associations by resolved, case-insensitive file path. */
function samePath(left, right) {
  return typeof left === "string" && typeof right === "string" && resolve(left).toLowerCase() === resolve(right).toLowerCase();
}

/** Decode a PNG to four raw channels for exact RGB/alpha comparison. */
async function decodePng(path) {
  const { data, info } = await sharedSharp(await readFile(path), { ignoreIcc: true }).ensureAlpha().raw().toBuffer({ resolveWithObject: true });
  assert.equal(info.channels, 4);
  return { width: info.width, height: info.height, data };
}

/**
 * Compare two PNGs and optionally save a diagnostic diff: red is visible RGB
 * delta and blue is alpha delta. The diff never changes the strict verdict.
 */
async function compareFiles(left, right, diffPath) {
  const expected = await decodePng(left), actual = await decodePng(right);
  const result = compareRgba(expected, actual);
  if (diffPath && result.sameSize) {
    const pixels = Buffer.alloc(expected.width * expected.height * 4);
    for (let i = 0; i < expected.width * expected.height; i++) {
      const p = i * 4;
      const alpha = Math.abs(expected.data[p + 3] - actual.data[p + 3]);
      const visible = expected.data[p + 3] !== 0 || actual.data[p + 3] !== 0;
      pixels[p] = visible ? Math.max(...[0, 1, 2].map(k => Math.abs(expected.data[p + k] - actual.data[p + k]))) : 0;
      pixels[p + 1] = 0;
      pixels[p + 2] = alpha;
      pixels[p + 3] = 255;
    }
    await sharedSharp(pixels, { raw: { width: expected.width, height: expected.height, channels: 4 } }).png().toFile(diffPath);
  }
  return result;
}

/** Inspect the imported PSD, render it with the selected CLI, and retain both outputs. */
async function runCli(cli, source, output, work) {
  const inspect = await exec(cli, ["run", source, "--cmd", "document.inspect"]);
  await writeFile(join(work, `inspect-${output === "photocraft.png" ? "adjusted" : "baseline"}.json`), inspect.stdout + "\n");
  const record = JSON.parse(inspect.stdout.split(/\r?\n/).find(s => s.includes('"command":"document.inspect"')));
  assert.equal(record.command, "document.inspect");
  assert(record.result?.layers?.length, "PhotoCraft imported no layers");
  await exec(cli, ["convert", source, join(work, output)]);
  assert((await stat(join(work, output))).size > 0, "PhotoCraft produced an empty PNG");
  return record.result;
}

/** Assert imported RGB/8 Hue/Saturation sliders, zero color ranges, clipping, and issue mask. */
function assertImportedStructure(info, c) {
  assert.equal(info.mode, "Rgb", "PhotoCraft did not import RGB mode");
  assert.equal(info.depth, 8, "PhotoCraft did not import 8-bit depth");
  const top = info.layers[0];
  assert.equal(top?.kind, "Adjustment", "PhotoCraft lost the adjustment layer");
  const params = top.adjustment?.HueSaturation ??
    (top.adjustment?.kind === "hueSaturation" ? top.adjustment.params : undefined);
  assert(params, "PhotoCraft did not retain Hue/Saturation settings");
  assert.equal(params.hue, c.hue);
  assert.equal(params.saturation, c.saturation);
  assert.equal(params.lightness, c.lightness);
  assert.equal(params.colorize, false);
  const rangeNames = ["reds", "yellows", "greens", "cyans", "blues", "magentas"];
  if (params.ranges) assert.equal(params.ranges.length, 6, "PhotoCraft lost Hue/Saturation color ranges");
  for (const [index, range] of rangeNames.entries())
    for (const slider of ["hue", "saturation", "lightness"])
      assert.equal((params.ranges?.[index] ?? params[range])?.[slider], 0, `PhotoCraft ${range}.${slider} differs`);
  assert.equal(top.clipped, c.clipped, "PhotoCraft changed clipping");
  if (c.kind === "issue") assert.equal(top.hasMask, true, "PhotoCraft lost adjustment mask");
}

/** Accept either SDK-decoded JSX objects or its JSON-string transport form. */
function parseJsx(value) {
  return typeof value === "string" ? JSON.parse(value) : value;
}

/** Own one Bridge session, its isolated Photoshop documents, and parity artifacts. */
export class LiveHarness {
  /** Record the toolkit's live Bridge without opening or changing a document. */
  constructor(bridge) {
    this.bridge = bridge;
    this.connection = undefined;
    this.owned = new Set();
    this.ownedPaths = new Map();
    this.closeFailures = new Map();
    this.cli = resolve(process.env.PHOTOCRAFT_CLI ?? join(repoRoot, "target", "debug", "photocraft-cli.exe"));
    this.provenance = undefined;
  }

  /** Connect to the running host and write binary/source/toolkit provenance. */
  async start() {
    assert(existsSync(this.cli), `NOT_EXECUTED: CLI is missing: ${this.cli}`);
    await mkdir(artifactsRoot, { recursive: true });
    sharedSharp = loadSharedSharp();
    this.provenance = await sourceProvenance(this.cli);
    const options = getGeneratorConnectionOptions();
    this.connection = new RawConnection({ ...options, WebSocket, maxRetries: 0 });
    await this.connection.ready();
    const sdkInfo = await this.connection.getServerInfo();
    const bridgeVersion = await this.bridge.generator.getPhotoshopVersion();
    assert.equal(sdkInfo.psVersion, bridgeVersion, "SDK and in-process Bridge disagree on Photoshop version");
    this.provenance.photoshopVersion = bridgeVersion;
    this.provenance.bridgeServer = { psVersion: sdkInfo.psVersion };
    await writeFile(join(artifactsRoot, "provenance.json"), JSON.stringify(this.provenance, null, 2) + "\n");
  }

  /** Read current document IDs, paths, dirty flags, and active ID for user-state checks. */
  async state() {
    const [list, active] = await Promise.all([
      this.connection.invoke("document/list", {}),
      this.connection.invoke("document/active", {}),
    ]);
    return documentState(list, active);
  }

  /**
   * Open a per-case copied path and claim only its new ID. If the open response
   * is lost, reconcile by the unique path before attempting owned cleanup.
   */
  async openOwned(path, before) {
    let opened;
    try {
      opened = await this.connection.invoke("document/open", { filePath: path });
      assertOwned(opened, before, path);
      this.registerOwned(opened.id, path);
      return opened.id;
    } catch (error) {
      // An uncertain open can leave a new document. Claim only a unique file whose
      // path is our freshly-created per-case path; never claim a user ID.
      const after = await this.state();
      const candidate = after.documents.filter(d => !before.documents.some(b => b.id === d.id) && samePath(d.filePath, path));
      if (candidate.length === 1) {
        this.registerOwned(candidate[0].id, path);
        await this.closeOwned(candidate[0].id);
      }
      throw error;
    }
  }

  /** Register a new test-owned ID and its copied source path for later close checks. */
  registerOwned(id, path) {
    assert(Number.isSafeInteger(id) && id > 0, "Cannot register an invalid Photoshop document ID");
    assert(!this.owned.has(id), "Photoshop document ID is already registered as test-owned");
    this.owned.add(id);
    this.ownedPaths.set(id, new Set([resolve(path).toLowerCase()]));
  }

  /** Permit a new test-owned association while retaining the old copied path. */
  expectOwnedPath(id, path) {
    assert(this.owned.has(id), "Cannot change path of a document not registered as test-owned");
    this.ownedPaths.get(id).add(resolve(path).toLowerCase());
  }

  /**
   * Register the target path before non-idempotent saveAs: Photoshop may save
   * successfully even if the transport response is lost. Never auto-retry it.
   */
  async saveOwnedAs(id, path) {
    // Photoshop may commit the new association even when the response is lost.
    // Record this unique test-owned path before invoking the non-idempotent save.
    this.expectOwnedPath(id, path);
    return this.connection.invoke("document/saveAs", { documentId: id, filePath: path });
  }

  /**
   * Close only an owned ID still associated with a known per-case path. On an
   * uncertain close response, list documents once and forbid automatic replay.
   */
  async closeOwned(id) {
    assert(this.owned.has(id), "Refusing to close a document not registered as test-owned");
    if (this.closeFailures.has(id)) throw this.closeFailures.get(id);
    const docs = await this.connection.invoke("document/list", {});
    const current = docs.find(d => d.id === id);
    if (!current) {
      this.owned.delete(id);
      this.ownedPaths.delete(id);
      return;
    }
    assert(this.ownedPaths.get(id)?.has(resolve(current.filePath ?? "").toLowerCase()),
      `Refusing to close Photoshop document ${id}: its current file path is not test-owned`);
    try {
      await this.connection.invoke("document/close", { documentId: id, save: "discard" });
      this.owned.delete(id);
      this.ownedPaths.delete(id);
    } catch (error) {
      let after;
      try { after = await this.connection.invoke("document/list", {}); }
      catch (listError) {
        const failure = new Error(`Photoshop close response failed for ${id}; document state is unknown: ${error.message}; list failed: ${listError.message}`);
        this.closeFailures.set(id, failure);
        throw failure;
      }
      if (!after.some(d => d.id === id)) {
        this.owned.delete(id);
        this.ownedPaths.delete(id);
        throw new Error(`Photoshop close response failed for ${id}, but the document is absent: ${error.message}`);
      }
      const failure = new Error(`Photoshop close response failed for ${id}; document is still open and automatic retry is unsafe: ${error.message}`);
      this.closeFailures.set(id, failure);
      throw failure;
    }
  }

  /**
   * Build or copy a case PSD, prove Photoshop setup/import, compare exact PNGs,
   * and persist the verdict while restoring pre-existing host document state.
   */
  async execute(c) {
    const work = join(artifactsRoot, c.id);
    await mkdir(work, { recursive: true });
    const result = { schemaVersion: 1, id: c.id, exploratory: !!c.exploratory, status: "NOT_EXECUTED", work, provenance: "../provenance.json", comparisons: {} };
    let before, ownedId;
    let failure;
    try {
      before = await this.state();
      result.userStateBefore = before;
      const sourcePath = c.kind === "issue" ? await this.issueSource(c, work) :
        c.kind === "source-image" ? await this.sourceImage(work) : await this.syntheticSource(c, work);
      result.sourceSha256 = await sha256(sourcePath);
      if (c.kind === "issue") assert.equal(result.sourceSha256, "65c3a3110794ec264e87a51c2e9dfd0b920106890f50feb73d0d1c77c10dc2f4", "SETUP_UNVERIFIED: issue PSD hash changed");
      ownedId = await this.openOwned(sourcePath, before);
      result.ownedDocumentId = ownedId;
      if (c.kind === "issue") {
        result.psReadback = parseJsx(await this.connection.invoke("jsx/run", {
          documentId: ownedId, script: inspectIssueScript(ownedId), historyName: "Read HS fixture",
        }));
        assert.equal(result.psReadback.width, 1662);
        assert.equal(result.psReadback.height, 1026);
        assert.equal(result.psReadback.layerCount, 2);
      } else {
        result.sourceReadback = parseJsx(await this.connection.invoke("jsx/run", {
          documentId: ownedId,
          script: c.kind === "source-image" ? prepareIssueBaseScript(ownedId) : prepareSourceScript(ownedId),
          historyName: "Prepare HS source",
        }));
        assert.equal(result.sourceReadback.documentId, ownedId);
        assert.equal(result.sourceReadback.background, false);
        await this.saveOwnedAs(ownedId, join(work, "baseline.psd"));
        await this.connection.invoke("document/export", { documentId: ownedId, filePath: join(work, "baseline-ps.png"), format: "png" });
        const baselineInfo = await runCli(this.cli, join(work, "baseline.psd"), "baseline-photocraft.png", work);
        assert.equal(baselineInfo.mode, "Rgb");
        assert.equal(baselineInfo.depth, 8);
        result.comparisons.importBaseline = await compareFiles(join(work, "baseline-ps.png"), join(work, "baseline-photocraft.png"));
        assert.equal(result.comparisons.importBaseline.exact, true, "SETUP_UNVERIFIED: PS/PhotoCraft baseline import differs");
        result.psReadback = parseJsx(await this.connection.invoke("jsx/run", {
          documentId: ownedId, script: adjustmentScript({ ...c, caseId: c.id, documentId: ownedId }), historyName: "Create HS test layer",
        }));
        await this.saveOwnedAs(ownedId, join(work, "input.psd"));
      }
      assert.equal(result.psReadback.documentId, ownedId);
      assert.equal(result.psReadback.hue, c.hue);
      assert.equal(result.psReadback.saturation, c.saturation);
      assert.equal(result.psReadback.lightness, c.lightness);
      assert.equal(result.psReadback.colorize, false);
      assert(c.identity ? result.psReadback.adjustmentRecords <= 1 : result.psReadback.adjustmentRecords === 1,
        "SETUP_UNVERIFIED: Photoshop has non-master Hue/Saturation ranges");
      assert.equal(result.psReadback.clipped, c.clipped);
      assert(["sRGB", "sRGB IEC61966-2.1"].includes(result.psReadback.profile), "SETUP_UNVERIFIED: Photoshop profile changed");
      if (c.kind === "issue") assert.equal(result.psReadback.hasUserMask, true, "SETUP_UNVERIFIED: issue adjustment mask missing");
      await this.connection.invoke("document/export", { documentId: ownedId, filePath: join(work, "photoshop.png"), format: "png" });
      result.photocraftInspect = await runCli(this.cli, join(work, "input.psd"), "photocraft.png", work);
      assertImportedStructure(result.photocraftInspect, c);
      if (c.kind === "issue") {
        const issueRoot = process.env.PHOTOCRAFT_HS_ISSUE_ROOT;
        const archived = requiredArchivedReference(issueRoot);
        result.comparisons.archivedPhotoshop = await compareFiles(archived, join(work, "photoshop.png"));
        assert.equal(result.comparisons.archivedPhotoshop.exact, true, "SETUP_UNVERIFIED: live Photoshop no longer matches archived issue reference");
        result.comparisons.photoshopImpact = await compareFiles(join(issueRoot, issueRelative, "ps-import-baseline.png"), join(work, "photoshop.png"));
        assertPhotoshopImpact(result.comparisons.photoshopImpact, c);
      } else {
        result.comparisons.photoshopImpact = await compareFiles(join(work, "baseline-ps.png"), join(work, "photoshop.png"));
        assertPhotoshopImpact(result.comparisons.photoshopImpact, c);
      }
      result.comparisons.parity = await compareFiles(join(work, "photoshop.png"), join(work, "photocraft.png"), join(work, "rgb-alpha-diff.png"));
      assert.equal(result.comparisons.parity.exact, true, `PIXEL_MISMATCH: ${result.comparisons.parity.rgbPixels} RGB pixels, ${result.comparisons.parity.alphaPixels} alpha pixels, max RGB delta ${result.comparisons.parity.maxRgbDifference}`);
      result.status = "PASS";
    } catch (error) {
      failure = error;
      result.status = String(error.message).startsWith("NOT_EXECUTED") ? "NOT_EXECUTED" : "FAIL";
      result.error = { name: error.name, message: error.message, code: error.code, source: error.source, details: error.details };
    } finally {
      if (ownedId !== undefined && this.owned.has(ownedId)) {
        try { await this.closeOwned(ownedId); }
        catch (error) { failure ??= error; result.status = "FAIL"; result.cleanupError = error.message; }
      }
      if (before) {
        try {
          result.userStateAfter = await this.state();
          assert.deepEqual(result.userStateAfter, before, "USER_STATE_CHANGED: pre-existing Photoshop documents or active document changed");
        } catch (error) { failure ??= error; result.status = "FAIL"; result.userStateError = error.message; }
      }
      await writeFile(join(work, "case.json"), JSON.stringify(result, null, 2) + "\n");
    }
    if (failure) throw failure;
    return result;
  }

  /** Copy the immutable issue PSD into a per-case path without editing the source. */
  async issueSource(c, work) {
    const root = process.env.PHOTOCRAFT_HS_ISSUE_ROOT;
    assert(root, "NOT_EXECUTED: set PHOTOCRAFT_HS_ISSUE_ROOT to the external issue folder");
    const original = join(resolve(root), issueRelative, "input.psd");
    assert(existsSync(original), `NOT_EXECUTED: required issue PSD missing: ${original}`);
    const copy = join(work, "input.psd");
    await copyFile(original, copy);
    return copy;
  }

  /** Render opaque sRGB synthetic pixels for one diagnostic Photoshop case. */
  async syntheticSource(c, work) {
    const { width, height, data } = rasterFor(c.kind);
    const png = join(work, "source.png");
    await sharedSharp(data, { raw: { width, height, channels: 4 } }).withMetadata({ icc: "srgb" }).png().toFile(png);
    return png;
  }

  /** Copy the verified issue PSD for isolated slider cases on its source image. */
  async sourceImage(work) {
    const root = process.env.PHOTOCRAFT_HS_ISSUE_ROOT;
    assert(root, "NOT_EXECUTED: set PHOTOCRAFT_HS_ISSUE_ROOT to the external issue folder");
    const original = join(resolve(root), issueRelative, "input.psd");
    assert(existsSync(original), `NOT_EXECUTED: required issue PSD missing: ${original}`);
    assert.equal(await sha256(original), "65c3a3110794ec264e87a51c2e9dfd0b920106890f50feb73d0d1c77c10dc2f4", "SETUP_UNVERIFIED: issue PSD hash changed");
    const copy = join(work, "source.psd");
    await copyFile(original, copy);
    return copy;
  }

  /** Close any remaining owned IDs once, then release the Bridge connection. */
  async finish() {
    const leftovers = [...this.owned];
    const errors = [];
    for (const id of leftovers) try { await this.closeOwned(id); } catch (error) { errors.push(`${id}: ${error.message}`); }
    this.connection?.close();
    if (errors.length) throw new Error(`Owned Photoshop document cleanup failed: ${errors.join("; ")}`);
  }
}
