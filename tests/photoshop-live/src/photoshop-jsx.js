/**
 * Build JSX that validates a test-owned RGB/8 source document, removes a
 * background-layer flag if present, and returns its profile and document ID.
 * The caller owns opening, saving, and closing the document.
 * @param {number} documentId Expected Photoshop document ID.
 * @returns {string} Script for jsx/run's explicit documentId.
 */
export function prepareSourceScript(documentId) {
  return `
(function (ownedId) {
  var doc = app.activeDocument;
  if (doc.id !== ownedId) throw new Error("JSX did not target owned document");
  if (doc.mode !== DocumentMode.RGB || doc.bitsPerChannel !== BitsPerChannelType.EIGHT)
    throw new Error("Expected RGB/8 input document");
  if (doc.activeLayer.isBackgroundLayer) doc.activeLayer.isBackgroundLayer = false;
  if (doc.colorProfileName.indexOf("sRGB") < 0) throw new Error("Expected sRGB source profile");
  return JSON.stringify({ documentId: doc.id, profile: doc.colorProfileName,
    background: doc.activeLayer.isBackgroundLayer });
})(${JSON.stringify(documentId)});`;
}

/**
 * Build JSX that removes the adjustment from a test-owned copy of the issue
 * PSD, producing its source-layer baseline without touching a user document.
 * @param {number} documentId Expected Photoshop document ID.
 * @returns {string} Script for jsx/run's explicit documentId.
 */
export function prepareIssueBaseScript(documentId) {
  return `
(function (ownedId) {
  var doc = app.activeDocument;
  if (doc.id !== ownedId) throw new Error("JSX did not target owned document");
  if (doc.mode !== DocumentMode.RGB || doc.bitsPerChannel !== BitsPerChannelType.EIGHT)
    throw new Error("Expected RGB/8 input document");
  if (doc.layers.length !== 2) throw new Error("Issue PSD layer count changed");
  doc.layers[0].remove();
  doc.activeLayer = doc.layers[0];
  if (doc.colorProfileName.indexOf("sRGB") < 0) throw new Error("Expected sRGB source profile");
  return JSON.stringify({ documentId: doc.id, profile: doc.colorProfileName,
    background: doc.activeLayer.isBackgroundLayer });
})(${JSON.stringify(documentId)});`;
}

/**
 * Build JSX that creates one clipped Master Hue/Saturation layer with the
 * requested sliders and returns Photoshop's serialized adjustment readback.
 * The script refuses the wrong document ID, color mode, or bit depth.
 * @param {{documentId: number, caseId: string, hue: number, saturation: number, lightness: number, clipped: boolean}} config
 * @returns {string} Script for jsx/run's explicit documentId.
 */
export function adjustmentScript(config) {
  const data = JSON.stringify(config);
  return `
(function (p) {
  var s = stringIDToTypeID;
  var doc = app.activeDocument;
  if (doc.id !== p.documentId) throw new Error("JSX did not target owned document");
  if (doc.mode !== DocumentMode.RGB || doc.bitsPerChannel !== BitsPerChannelType.EIGHT)
    throw new Error("Expected RGB/8 input document");
  var make = new ActionDescriptor(), ref = new ActionReference();
  ref.putClass(s("adjustmentLayer"));
  make.putReference(s("null"), ref);
  var settings = new ActionDescriptor();
  settings.putEnumerated(s("presetKind"), s("presetKindType"), s("presetKindCustom"));
  settings.putBoolean(s("colorize"), false);
  var master = new ActionDescriptor(), adjustments = new ActionList();
  master.putInteger(s("hue"), p.hue);
  master.putInteger(s("saturation"), p.saturation);
  master.putInteger(s("lightness"), p.lightness);
  adjustments.putObject(s("hueSatAdjustmentV2"), master);
  settings.putList(s("adjustment"), adjustments);
  var using = new ActionDescriptor();
  using.putString(s("name"), "hs-live-" + p.caseId);
  using.putObject(s("type"), s("hueSaturation"), settings);
  make.putObject(s("using"), s("adjustmentLayer"), using);
  executeAction(s("make"), make, DialogModes.NO);
  var layer = doc.activeLayer;
  if (p.clipped) layer.grouped = true;
  var dref = new ActionReference();
  dref.putIdentifier(s("layer"), layer.id);
  var d = executeActionGet(dref);
  var adjustment = d.getList(s("adjustment")).getObjectValue(0);
  var records = adjustment.hasKey(s("adjustment")) ? adjustment.getList(s("adjustment")) : null;
  if (!records && (p.hue !== 0 || p.saturation !== 0 || p.lightness !== 0))
    throw new Error("Photoshop omitted a nonidentity adjustment record");
  var masterReadback = records ? records.getObjectValue(0) : null;
  return JSON.stringify({
    documentId: doc.id, layerId: layer.id, width: doc.width.as("px"), height: doc.height.as("px"),
    profile: doc.colorProfileName, mode: String(doc.mode), depth: String(doc.bitsPerChannel),
    hue: masterReadback ? masterReadback.getInteger(s("hue")) : 0,
    saturation: masterReadback ? masterReadback.getInteger(s("saturation")) : 0,
    lightness: masterReadback ? masterReadback.getInteger(s("lightness")) : 0,
    colorize: adjustment.hasKey(s("colorize")) && adjustment.getBoolean(s("colorize")),
    adjustmentRecords: records ? records.count : 0,
    clipped: layer.grouped,
    hasUserMask: d.hasKey(s("hasUserMask")) && d.getBoolean(s("hasUserMask"))
  });
})(${data});`;
}

/**
 * Build read-only JSX for the archived issue layer, including Master sliders,
 * clipping, profile, dimensions, and the original user-mask presence.
 * @param {number} documentId Expected Photoshop document ID.
 * @returns {string} Script for jsx/run's explicit documentId.
 */
export function inspectIssueScript(documentId) {
  return `
(function (ownedId) {
  var s = stringIDToTypeID, doc = app.activeDocument;
  if (doc.id !== ownedId) throw new Error("JSX did not target owned document");
  if (doc.mode !== DocumentMode.RGB || doc.bitsPerChannel !== BitsPerChannelType.EIGHT)
    throw new Error("Issue fixture is not RGB/8");
  var layer = doc.layers[0], r = new ActionReference();
  r.putIdentifier(s("layer"), layer.id);
  var d = executeActionGet(r), a = d.getList(s("adjustment")).getObjectValue(0);
  var records = a.getList(s("adjustment")), master = records.getObjectValue(0);
  return JSON.stringify({ documentId: doc.id, layerId: layer.id, layerCount: doc.layers.length,
    width: doc.width.as("px"), height: doc.height.as("px"), profile: doc.colorProfileName,
    hue: master.getInteger(s("hue")), saturation: master.getInteger(s("saturation")),
    lightness: master.getInteger(s("lightness")), colorize: a.getBoolean(s("colorize")),
    adjustmentRecords: records.count,
    clipped: layer.grouped, hasUserMask: d.hasKey(s("hasUserMask")) && d.getBoolean(s("hasUserMask")) });
})(${JSON.stringify(documentId)});`;
}
