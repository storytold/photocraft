export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const TOOL_TITLES = {
  session_list: "查看会话",
  doc_open: "打开文件",
  doc_new: "新建画布",
  doc_place: "置入图层",
  doc_replace_smart_object: "替换智能对象",
  doc_export_layers: "导出图层",
  doc_export_data_sets: "导出数据组",
  doc_import_data_sets: "导入数据组",
  doc_save: "保存文档",
  doc_export: "导出文档",
  doc_inspect: "检查图层",
  doc_render_preview: "生成预览",
  doc_select: "切换文档",
  doc_close: "关闭文档",
  command_list: "查询可用命令",
  command_run: "执行命令",
  command_batch: "批量执行",
  jobs_list: "查看任务",
  jobs_cancel: "取消任务",
  "ui.move": "移动鼠标",
  "ui.click": "鼠标点击",
  "ui.pointer": "画布指针",
  "ui_set": "切换工具",
  "ui_menu_invoke": "菜单操作",
  "ui.dialog.set": "填写弹窗",
  "ui.dialog.confirm": "确认弹窗",
  "ui.run_command": "执行命令",
  "ui.inspect": "读取界面",
};

export function commandTitle(id = "") {
  if (id.includes("newFillLayer.solidColor")) return "创建填充图层";
  if (id.includes("newFillLayer.gradient")) return "创建渐变填充图层";
  if (id === "edit.fill") return "填充颜色";
  if (id.includes("gaussianBlur")) return "高斯模糊";
  if (id.includes("sharpen")) return "锐化";
  if (id.includes("curves")) return "调整曲线";
  if (id.includes("document.inspect")) return "检查文档";
  if (id.includes("layer.new.layer")) return "新建图层";
  if (id.includes("layer.new.group")) return "新建图层组";
  if (id.includes("resize")) return "调整尺寸";
  return id || "执行命令";
}

export function stepTitle(step) {
  if (step.label) return step.label;
  if (step.tool === "command_run") return commandTitle(step.arguments?.id);
  if (step.tool === "doc_inspect" && step.auto) return "自动检查图层";
  return TOOL_TITLES[step.tool] || step.tool;
}

export function stepStatus(step) {
  if (step.is_error) return `${stepTitle(step)}失败`;
  return `正在${stepTitle(step)}…`;
}

export function summarizeStep(step) {
  const result = step.result;
  if (step.tool === "ui.click" || step.tool === "ui.move" || step.tool === "ui.dialog.set") {
    return typeof result === "string" ? result : "完成";
  }
  if (step.tool === "ui.inspect" && result && typeof result === "object") {
    if (Array.isArray(result.layers)) {
      return `${result.width ?? "?"} × ${result.height ?? "?"} px · 图层：${result.layers.join("、") || "无"}`;
    }
    if (result.window) {
      return `窗口 ${Math.round(result.window.width)} × ${Math.round(result.window.height)} · 工具 ${result.tool || "?"}`;
    }
  }
  if (step.is_error) {
    return (result && result.error) || "工具返回了错误";
  }
  if (step.tool === "doc_inspect" && result && typeof result === "object") {
    const layers = Array.isArray(result.layers) ? result.layers : [];
    const names = layers.map((layer) => layer.name).filter(Boolean);
    const suffix = names.length ? `：${names.slice(0, 4).join("、")}${names.length > 4 ? "…" : ""}` : "";
    return `${result.width ?? "?"} × ${result.height ?? "?"} px · ${layers.length} 个图层${suffix}`;
  }
  if (step.tool === "doc_new" && result) {
    return `文档 #${result.document ?? 0} 已创建`;
  }
  if (step.tool === "command_list" && result) {
    return `找到 ${result.count ?? 0} 个可用命令`;
  }
  if (step.tool === "command_run") {
    return `命令 ${step.arguments?.id || ""} 执行成功`;
  }
  if (step.tool === "doc_render_preview") {
    return result?.preview_png ? "预览 PNG 已生成" : "预览已生成";
  }
  if (step.tool === "session_list" && result) {
    return `当前打开 ${Array.isArray(result.documents) ? result.documents.length : 0} 个文档`;
  }
  if (typeof result === "string") return result.slice(0, 160);
  if (result && typeof result === "object") {
    const compact = JSON.stringify(result);
    return compact.length > 180 ? `${compact.slice(0, 180)}…` : compact;
  }
  return "完成";
}

export function inferFillColor(step) {
  if (!step || step.is_error) return null;
  const args = step.arguments || {};
  if (step.tool === "doc_new" && /^#[0-9a-fA-F]{6}$/.test(args.background || "")) {
    return args.background;
  }
  if (step.tool === "command_run") {
    const id = args.id || "";
    if (id.includes("newFillLayer") && /^#[0-9a-fA-F]{6}$/.test(args.params?.color || "")) {
      return args.params.color;
    }
    if (id === "edit.fill" && /^#[0-9a-fA-F]{6}$/.test(args.params?.color || "")) {
      return args.params.color;
    }
  }
  if (step.tool === "command_batch" && Array.isArray(args.steps)) {
    for (const inner of args.steps) {
      const color = inferFillColor({
        tool: "command_run",
        is_error: false,
        arguments: inner,
      });
      if (color) return color;
    }
  }
  return null;
}

export function inferFillFromLogs(steps = []) {
  for (let i = steps.length - 1; i >= 0; i -= 1) {
    const color = inferFillColor(steps[i]);
    if (color) return color;
  }
  return null;
}

export function colorName(hex) {
  const names = {
    "#0000ff": "蓝色",
    "#3b82f6": "蓝色",
    "#ff0000": "红色",
    "#00ff00": "绿色",
    "#000000": "黑色",
    "#ffffff": "白色",
    "#ffff00": "黄色",
    "#800080": "紫色",
    "#ffa500": "橙色",
  };
  return names[String(hex).toLowerCase()] || hex;
}

export function pretty(value, limit = 3200) {
  let text;
  try {
    text = JSON.stringify(value, null, 2);
  } catch {
    text = String(value);
  }
  if (text == null) return "";
  if (text.length <= limit) return text;
  return `${text.slice(0, limit)}\n…（已截断）`;
}

export function hasArgs(step) {
  const args = step?.arguments;
  return args && typeof args === "object" && Object.keys(args).length > 0;
}
