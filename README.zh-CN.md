> 本文档是 [README.md](README.md) 的中文翻译，若两者有出入以英文原版为准。

<p align="center">
  <a href="https://getartcraft.com/">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="docs/brand/artcraft-logo-white.svg">
      <img alt="ArtCraft" src="docs/brand/artcraft-logo.svg" width="200">
    </picture>
  </a>
</p>

<p align="center">
  <img src="assets/app-icon/hicolor/256x256/apps/ai.storyteller.photocraft.png" width="128" height="128" alt="PhotoCraft 图标：蓝色瓷砖上雕刻的九尾狐">
</p>

<h1 align="center">PhotoCraft</h1>

<p align="center">
  <b>图像编辑；以纯 Rust 重建的 Adobe Photoshop 开源净室复现。</b><br>
  图层、蒙版、调整图层、图层样式、文字、矢量、画笔和真实的 PSD 文件，<br>
  尽在一款完全用 Rust 编写的原生应用中。开源、离线、属于你。
</p>

<p align="center">
  <img alt="100% Rust" src="https://img.shields.io/badge/100%25-Rust-b7410e?style=flat-square&logo=rust">
  <img alt="macOS · Windows · Linux · FreeBSD · Web" src="https://img.shields.io/badge/macOS%20%C2%B7%20Windows%20%C2%B7%20Linux%20%C2%B7%20FreeBSD%20%C2%B7%20Web-native-2f7bf5?style=flat-square">
  <img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-3a3a3a?style=flat-square">
  <img alt="Status: early alpha" src="https://img.shields.io/badge/status-early%20alpha-d69e2e?style=flat-square">
  <br>
  <a href="https://sonarcloud.io/project/overview?id=storytold_photocraft"><img alt="Scanned by SonarCloud" src="https://img.shields.io/badge/Scanned_by-SonarCloud-4b9fd5?style=flat-square&logo=sonarqubecloud&logoColor=white"></a>
  <a href="https://coderabbit.ai"><img alt="Reviewed by CodeRabbit" src="https://img.shields.io/coderabbit/prs/github/storytold/photocraft?style=flat-square&label=Reviewed%20by%20CodeRabbit&logo=coderabbit"></a>
</p>

<p align="center">
  <a href="https://discord.gg/artcraft"><img alt="Join the ArtCraft community on Discord" src="https://img.shields.io/badge/Join%20us%20on%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="40"></a>
</p>

<p align="center">
  <a href="https://getartcraft.com/apps/photocraft"><b>getartcraft.com 上的 PhotoCraft</b></a> ·
  <a href="https://getartcraft.com/">ArtCraft</a> ·
  <a href="https://getartcraft.com/apps">全部 Crafting Apps</a> ·
  <a href="README.md">English</a>
</p>

<br>

<p align="center">
  <img src="docs/images/photocraft-demo.jpg" alt="PhotoCraft 正在编辑葛饰北斋的《神奈川冲浪里》：带投影的字幕卡片、Title 和 Credit 文字图层、自然饱和度与曲线调整图层，以及绘制在图像直方图上的曲线编辑器" width="100%">
  <br>
  <sub>带投影的字幕卡片、实时文字，以及自然饱和度和曲线调整图层，曲线编辑器处于打开状态。<br>
  《<i>神奈川冲浪里</i>》，葛饰北斋，约 1831 年</sub>
</p>

> [!NOTE]
> **ArtCraft 是一个汇聚各行各业艺术家的社区。** 数字艺术、生成艺术、音乐、
> 游戏 &mdash; 只要你在创作，你就是我们的一员。**[来 Discord 打个招呼吧](https://discord.gg/artcraft)。**

<p align="center">
  <a href="#功能特性">功能特性</a> ·
  <a href="#开箱即用的全部功能">开箱即用</a> ·
  <a href="#毫不妥协的-psd-支持">PSD</a> ·
  <a href="#为-agent-而生">Agent</a> ·
  <a href="#内部机制">内部机制</a> ·
  <a href="#快速上手">快速上手</a> ·
  <a href="#下载">下载</a> ·
  <a href="#crafting-apps-全家桶">Crafting Apps</a> ·
  <a href="https://discord.gg/artcraft">Discord</a>
</p>

<br>

<table>
  <tr>
    <td width="25%" valign="top">
      <h3>🎛️ 上手即熟悉</h3>
      菜单、快捷键、面板和工具都在你的双手习惯的位置，从 ⌘J 到 ⇧⌘D。如果你会用 Photoshop，你就已经会用 PhotoCraft。
    </td>
    <td width="25%" valign="top">
      <h3>⚡ 原生且快速</h3>
      基于 wgpu 的 GPU 合成器（Metal、Vulkan、DX12、WebGPU）、写时复制图块和多线程滤镜。没有 Electron，没有 web view，无需等待。
    </td>
    <td width="25%" valign="top">
      <h3>🗂️ 真实的 PSD 文件</h3>
      打开、编辑并保存分层的 Photoshop 文档。重新保存后，psd-tools 测试文件中 309 个里有 307 个渲染结果保持不变。
    </td>
    <td width="25%" valign="top">
      <h3>🤖 为 Agent 就绪</h3>
      每个操作都是一条命令，因此你可以通过 UI、CLI、JSON 控制通道或 MCP 服务器驱动同一个引擎。
    </td>
  </tr>
</table>

<br>

## 功能特性

这里的每张截图都是真实应用在处理公有领域艺术作品，通过其控制通道离屏渲染而成。

<table>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/images/photocraft-adjustments.jpg" alt="莫奈的《印象·日出》带有色阶和自然饱和度调整图层；右侧打开着曲线编辑器以及显示均值、标准差和中位数的直方图面板" width="100%">
      <br>
      <sub>色阶和自然饱和度调整图层，以及实时直方图面板。<br>《<i>印象·日出</i>》，克劳德·莫奈，1872 年</sub>
      <h3>编辑无后悔</h3>
      调整图层让每一次编辑都保持可修改。叠加色阶、曲线、自然饱和度、色相/饱和度等十几种调整，用蒙版限定区域、重新排序或关闭它们，你的原始像素永远不会改变。
      <br><br>
      <b>16 种调整图层</b>，也可直接作用于像素，包括支持逐通道编辑的曲线、带实时直方图的色阶、黑白、通道混合器、渐变映射、照片滤镜、可选颜色和颜色查找（.cube、.3dl、.look）。还有阴影/高光、替换颜色、匹配颜色、HDR 色调、去色和色调均化。
    </td>
    <td width="50%" valign="top">
      <img src="docs/images/photocraft-layer-styles.jpg" alt="图层样式对话框正在编辑 EARTHRISE 文字图层的外发光，同时启用了描边，背景是阿波罗 8 号拍摄的《地出》照片" width="100%">
      <br>
      <sub>实时文字图层上的外发光和描边，在图层样式对话框中。<br>《<i>地出</i>》，威廉·安德斯 / NASA，1968 年</sub>
      <h3>让作品出彩的样式</h3>
      投影、内阴影、外发光和内发光、斜面和浮雕、光泽、描边，以及颜色叠加、渐变叠加和图案叠加，实时作用于任何图层，包括文字图层。图案来自图案库（内置图案、编辑 › 定义图案、<code>.pat</code> 导入/导出）和 PSD <code>Patt</code> 块。
      <br><br>
      在图层之间复制粘贴样式、一键隐藏所有效果，并直接打开 PSD 中的样式，渲染效果与 Photoshop 一致。
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/images/photocraft-masks.jpg" alt="维米尔的《戴珍珠耳环的少女》：面部周围的椭圆选区，以及带椭圆蒙版的色相/饱和度图层，让面部以外的一切变成灰色" width="100%">
      <br>
      <sub>椭圆选区成为色相/饱和度图层的蒙版，因此只有面部保留色彩。<br>《<i>戴珍珠耳环的少女</i>》，约翰内斯·维米尔，约 1665 年</sub>
      <h3>懂你图像的选区</h3>
      选框、套索和魔棒用于精确选择；快速选择、对象选择和选择主体让计算机替你描边；选择并遮住用于细化发丝般的精细边缘。
      <br><br>
      羽化、扩展、收缩、平滑、扩大、重新选择。把任何选区变成图层蒙版、矢量路径或形状。智能选择在你的机器上运行，无需云端，无需账号。
    </td>
    <td width="50%" valign="top">
      <img src="docs/images/photocraft-type.jpg" alt="比尔施塔特的《内华达山脉之间》：画布上正在以 Georgia 字体编辑标题 SIERRA NEVADA，另有署名行和一段说明文字；字符和段落控件处于打开状态" width="100%">
      <br>
      <sub>就地编辑的标题，配以署名行和一段正文。<br>《<i>加利福尼亚，内华达山脉之间</i>》，阿尔伯特·比尔施塔特，1868 年</sub>
      <h3>排版优美的文字</h3>
      点文字和段落文字，直接在画布上编辑，配备完整的字符和段落控件：字体、字重、字号、行距、字距、对齐和颜色。<a href="docs/color-picker.md">完整的文字拾色器</a>还支持带像素放大镜的屏幕取色。
      <br><br>
      文字图层保持可编辑，可应用图层样式，并能在 PSD 中往返无损。
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/images/photocraft-vector.jpg" alt="由形状图层（Lotus、Water、Sun、Badge 和 Dotted Ring）构成的莲花徽章，叠加在莫奈的《睡莲》上，莲花路径的锚点处于选中状态" width="100%">
      <br>
      <sub>由形状图层构成的徽章：渐变填充的莲花、星形和点状圆环。<br>《<i>睡莲</i>》，克劳德·莫奈，1906 年</sub>
      <h3>像素级精确的矢量</h3>
      矩形、椭圆、三角形、多边形、直线，以及钢笔、自由钢笔和弯度钢笔工具，配合与分辨率无关的形状图层、渐变填充，以及虚线、对齐描边。
      <br><br>
      组合形状（合并、减去、相交、排除）、在路径面板中保存路径、用作矢量蒙版，或对其描边和填充。我们 PSD 语料库中 116 个形状图层全部与 Photoshop 的像素一致。
    </td>
    <td width="50%" valign="top">
      <img src="docs/images/photocraft-filters.jpg" alt="旋转扭曲滤镜对话框设为 320 度角，在梵高《星月夜》上的椭圆选区内实时预览" width="100%">
      <br>
      <sub>旋转扭曲在画布上实时预览，且仅作用于选区内部。<br>《<i>星月夜</i>》，文森特·梵高，1889 年</sub>
      <h3>先预览，再应用</h3>
      每个滤镜对话框都在画布上透过你的选区实时预览。模糊（高斯、方框、动感、径向、表面、智能、镜头、形状，以及模糊画廊：移轴、光圈、场景、旋转、路径）、锐化、减少杂色、扭曲（旋转扭曲、波浪、波纹、置换、切变、锯齿……）、像素化、风格化（油画、风、凸出……）、渲染（云彩、纤维、镜头光晕、光照效果）等等。
      <br><br>
      在智能对象上运行滤镜，它们保持可编辑：随时修改、隐藏、重新排序或加蒙版。
      <br><br>
      大半径模糊使用跨全部核心的滑动求和方框遍历：3.6 MP 图像上半径 180 的高斯模糊不到一秒。
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/images/photocraft-transform.jpg" alt="安塞尔·亚当斯《提顿山与蛇河》的旋转副本周围的自由变换手柄，历史记录面板列出打开、复制图层、自由变换等步骤" width="100%">
      <br>
      <sub>对旋转后的照片进行自由变换，每一步都列在历史记录中。<br>《<i>提顿山与蛇河</i>》，安塞尔·亚当斯，1942 年</sub>
      <h3>随心塑造形态</h3>
      自由变换支持缩放、旋转、斜切、扭曲和透视；精确的 90° 和 180° 旋转与翻转；再次变换。图层、文字、形状、蒙版和选区都能一起变换。
      <br><br>
      完整的历史记录、切换上一状态和历史记录画笔意味着每一步都可撤销，甚至一次一笔地撤销画笔笔触。
    </td>
    <td width="50%" valign="top">
      <img src="docs/images/photocraft-export-light.jpg" alt="浅色主题下的导出为对话框，背景是克里姆特的《吻》：JPG 质量 90、缩放至 50%，带预览和约 684K 的预估大小" width="100%">
      <br>
      <sub>浅色主题下的导出为，带预览和文件大小估算。<br>《<i>吻</i>》，古斯塔夫·克里姆特，1907–1908 年</sub>
      <h3>发布到任何地方</h3>
      导出为支持格式、质量、透明度和缩放，外加预览和即时文件大小估算。一键快速导出为 PNG。
      <br><br>
      可选深色 Pro 主题、轻盈的 Studio 主题，或经典外观。
    </td>
  </tr>
</table>

<br>

## 开箱即用的全部功能

<table>
  <tr>
    <td width="33%" valign="top">
      <h4>🧰 70 种工具</h4>
      移动 · 画板 · 矩形选框和椭圆选框 · 单行选框 · 单列选框 · 套索 · 多边形套索 · 磁性套索 · 对象选择 · 快速选择 · 魔棒 · 裁剪 · 透视裁剪 · 切片 · 切片选择 · 画框 · 吸管 · 颜色取样器 · 标尺 · 注释 · 计数 · 移除 · 污点修复画笔 · 修复画笔 · 修补 · 内容识别移动 · 红眼 · 画笔 · 铅笔 · 混合器画笔 · 颜色替换 · 仿制图章 · 图案图章 · 历史记录画笔 · 历史记录艺术画笔 · 橡皮擦 · 背景橡皮擦 · 魔术橡皮擦 · 渐变 · 油漆桶 · 模糊 · 锐化 · 涂抹 · 调整画笔 · 减淡 · 加深 · 海绵 · 钢笔 · 自由钢笔 · 弯度钢笔 · 添加锚点 · 删除锚点 · 转换点 · 横排和直排文字 · 横排和直排文字蒙版 · 路径选择 · 直接选择 · 矩形 · 椭圆 · 三角形 · 多边形 · 星形 · 直线 · 自定形状 · 抓手 · 旋转视图 · 缩放
    </td>
    <td width="33%" valign="top">
      <h4>🖌️ 真正的画笔引擎</h4>
      形状动态、散布、纹理、双重画笔、颜色动态、传递、画笔笔势、湿边、建立和平滑（包括拉绳效果），由压感笔的压力、倾斜、旋转和方向驱动。画笔预设、从选区定义画笔，以及确定性、可回放的笔触。
    </td>
    <td width="33%" valign="top">
      <h4>🗃️ 图层，做到位</h4>
      组、剪贴蒙版、像素和矢量蒙版、填充图层（纯色、渐变和图案）、调整图层、带智能滤镜及无损变换和变形的实时智能对象、多图层选择与对齐、分布和链接、Alpha 通道和快速蒙版、27 种 Photoshop 混合模式外加六种 Paint.NET 变体、不透明度和填充、锁定、颜色标签、图层类型过滤器、合并、拼合、栅格化、通过拷贝/剪切的图层、贴入。
    </td>
  </tr>
  <tr>
    <td width="33%" valign="top">
      <h4>🎨 任意色彩，任意深度</h4>
      RGB、灰度、CMYK 和 Lab 文档，每通道 8、16 和 32 位。位深度和色彩模型是运行时数据，因此每种工具在每种深度下都能工作。
      <br><br>
      纯 Rust 实现的真正 ICC 色彩管理：嵌入配置文件、指定和转换为配置文件（支持全部四种渲染意图和黑场补偿）、GPU 上的软打样（⌘Y）和色域警告（⇧⌘Y）。
    </td>
    <td width="33%" valign="top">
      <h4>🗂️ 格式</h4>
      PSD 和 PSB、分层 TIFF（TIFF 中的 Photoshop 图层数据，两种字节序均可读写）、<a href="docs/pdn.md">Paint.NET PDN3（读取，图层可编辑，支持全部 14 种混合模式）</a>、<a href="docs/ora.md">OpenRaster（读写，支持图层、组和混合模式，面向 Krita、MyPaint 和 GIMP）</a>、<a href="docs/xcf.md">GIMP XCF（读取，支持图层、组、蒙版、混合模式、通道和选区）</a>、<a href="docs/kra.md">Krita .kra（读取，支持绘画图层、组、混合模式和透明度蒙版）</a>、SVG（作为形状图层打开，作为矢量智能对象置入），以及平面 PNG、JPEG、TIFF、WebP（有损和无损）、GIF、BMP、TGA、ICO、QOI、PNM、OpenEXR、Radiance HDR 和 FITS（天文图像：8/16 位和 32 位浮点，灰度或 RGB，保留头信息），还有原生 <code>.pcraft</code> 格式。平面格式保留其支持的深度：PNG 保留 8/16 位样本，TIFF 保留 8/16 位和 32 位浮点样本，OpenEXR 保留半精度/浮点样本；JPEG、WebP 和其他 8 位格式会降低更高深度样本的精度并给出导出警告。各格式详见<a href="https://github.com/storytold/craft-libs/tree/main/crates/image-codecs#capability-matrix-default-build">编解码器能力矩阵</a>。来自 iPhone 和 Mac 的 HEIC 照片为只读（官方构建已包含；源码构建可选 <code>--features heif</code>）。AVIF 仅支持写入，需可选的 <code>avif</code> 构建特性；AVIF 解码和 JPEG XL 不可用。Affinity 文档（<code>.af</code>、<code>.afdesign</code>、<code>.afphoto</code>、<code>.afpub</code>）可原生只读打开，支持画板、图层、矢量、文字、图像和蒙版；效果、调整和其他不支持的部分会在警告中列出，Affinity 导出不可用。
    </td>
    <td width="33%" valign="top">
      <h4>🪄 日常必备</h4>
      自动色调、对比度和颜色 · 色调均化 · 图像和画布大小 · 裁剪和裁切 · 显示全部 · 编辑 › 填充和描边 · 合并拷贝 · 原位粘贴 · 参考线、标尺、网格和吸附 · 动作录制与回放 · 命令面板（⌘F）。
    </td>
  </tr>
</table>

<br>

## 毫不妥协的 PSD 支持

PhotoCraft 的 PSD 支持是一个独立 crate，依据 Adobe 公开规范编写，并用真实文件语料库进行测试。

- **忠实的往返：** 打开并重新保存文档后，psd-tools 测试集 309 个文件中 307 个、我们混合的 ag-psd/psd-tools 测试集 170 个中 169 个渲染结果相同（`crates/io/tests/it/corpus.rs`；用 `cargo xtask corpus --psd-tools` 获取 psd-tools 测试集），尚未建模的内容（原始块、描述符、附加数据）会被原样保留而不是丢弃。重新保存的文件与源文件并非逐字节相同：PhotoCraft 会重写图像资源、图层记录和合成图像。只有独立的 `craft-psd` crate（不经过文档模型直接解析和写入文件）能逐字节复现每一个可解析的语料库文件（`craft-psd/tests/it/corpus.rs`）。
- **像素级一致：** 合成图像基准将我们的渲染与 Photoshop 自身的合并图像对比，覆盖渐变插值（经典、可感知和线性）、图层效果、形状描边、剪贴和填充不透明度。
- **大型文档：** PSB、16 和 32 位文件，以及 CMYK 和 Lab 文档均可原生打开。

## 为 Agent 而生

每个菜单项、工具和对话框都运行来自同一个命令注册表（500+ 条命令）的命令。UI、CLI、JSON 控制通道和 MCP 服务器调用的都是同一批命令，因此凡是你能点击的，脚本或 AI agent 也能做到。

```sh
# Headless: open, edit, save
photocraft-cli run wave.psd \
  --cmd filter.sharpen.smartSharpen     --params '{"amount":80}' \
  --cmd layer.newAdjustmentLayer.curves --params '{"points":[[0,0],[64,48],[192,212],[255,255]]}' \
  --out wave-final.png

# Apply one action list to a folder of images
photocraft-cli batch --actions grade.json --in ./raw --out ./graded

# Every subcommand explains itself
photocraft-cli batch --help

# Let an agent drive it over MCP (headless, or bridged to the running app)
photocraft-cli mcp
```

桌面应用还提供经过认证、仅限回环地址的控制通道（`photocraft --control`），用于检查 UI 状态、以指针事件驱动工具，以及拍摄离屏截图。本 README 中的每张图片都是这样渲染的。参见 [`docs/control-protocol.md`](docs/control-protocol.md)。

## 内部机制

- **引擎优先：** 纯数据的文档模型加命令引擎，上层是轻量的 egui UI。分层在构建时强制执行，横跨 24 个 crate。
- **双合成器：** CPU 合成器作为参考基准，wgpu 合成器把画布放到 GPU 上。两者相互对照测试。
- **写时复制图块：** 256² 稀疏图块让撤销代价低廉、超大画布轻快，效果图按图层状态缓存。
- **浏览器中运行：** 整个引擎和 UI 都能编译为 WebAssembly。
- **净室实现：** 仅依据公开规范和观察到的行为实现。没有专有代码、着色器或资源。
- **经过测试：** 超过 1,700 个测试，包括 PSD 往返、合成生成器、合成器基准和多深度检查。

## 快速上手

```sh
git clone https://github.com/storytold/photocraft
cd photocraft
cargo run --release -p photocraft -- image.psd   # the desktop app
cargo test --workspace                           # the test suite
```

UI 和文字工具使用的日文字体来自 [craft-fonts](https://github.com/storytold/craft-fonts)，这是一个可选的构建输入（桌面发布构建始终包含它）。没有它时，PhotoCraft 会使用你系统的 CJK 字体：

```sh
git clone https://github.com/storytold/craft-fonts ../craft-fonts
CRAFT_FONTS_DIR="$PWD/../craft-fonts" cargo run --release -p photocraft
```

新贡献者和 AI agent：请先阅读 [`AGENTS.md`](AGENTS.md)，然后是 [`docs/`](docs/)。

macOS、Windows、Linux、FreeBSD 和 Web 的安装包附在每个 [GitHub release](https://github.com/storytold/photocraft/releases) 中。在 Linux 上你可以选择 AppImage、`.deb`、`.rpm`、tarball 或 Flatpak bundle。bundle 需要来自 [Flathub](https://flathub.org/setup) 的 freedesktop 运行时，`flatpak` 会提示随之一并安装：

```sh
flatpak install --user photocraft-<version>-linux-x86_64.flatpak   # or -linux-aarch64
flatpak run ai.storyteller.photocraft
```

Flatpak 通过主机的 CUPS 套接字打印（文件 › 打印直接以 IPP 与之通信，无需 `lp`）；没有 CUPS 或没有打印机时，文件 › 打印会通过打印门户打开系统打印对话框。参见[打印](docs/printing.md)。

AppImage 无需安装：首次运行会在 `~/.local/share` 注册启动器图标和菜单项，让 Wayland 下的 dock 显示 PhotoCraft 的图标。设置 `PHOTOCRAFT_NO_DESKTOP_INTEGRATION=1` 可跳过这一步，撤销方法见 [`docs/releasing.md`](docs/releasing.md) › Linux。

要使用内置 CLI，运行 `./photocraft-<version>-linux-x86_64.AppImage --cli --help`，或 `--cli mcp` 在 stdio 上启动 MCP 服务器。CLI 启动会跳过桌面集成。名为 `photocraft-cli` 的符号链接也会选择 CLI；参见 [`docs/development.md`](docs/development.md#headless-cli)。

在 Wayland 会话中，拖放到窗口的文件暂时无法打开：egui 底层的窗口库 winit 0.30 尚不支持 Wayland 拖放（#386）。请使用文件 › 打开，或在文件管理器中复制图像后用 Ctrl+V 粘贴。要使用拖放，请在 XWayland 下启动 PhotoCraft：`WAYLAND_DISPLAY= photocraft`、`WAYLAND_DISPLAY= ./photocraft-<version>-linux-x86_64.AppImage`，或 `flatpak run --nosocket=wayland --socket=x11 ai.storyteller.photocraft`。在 Wayland 上，PhotoCraft 会显示一条适用于你的安装方式的命令提示，直到你将其关闭。首选项 › 性能 › Linux 显示服务器 = X11 会在每次启动时起到同样作用。在小数显示缩放（125%、150%、175%）下，XWayland 窗口以较低分辨率绘制再由合成器放大，因此界面会模糊；原生 Wayland 保持清晰（#1072）。

在 macOS 上，命令行工具以 `photocraft-cli-<version>-macos-universal.zip` 提供。二进制文件使用与应用相同的 Developer ID 签名，并经 Apple 公证。裸二进制无法像 DMG 那样附带装订的公证票据，因此首次运行时 macOS 会在线检查公证。你可以自行验证：

```sh
ditto -x -k photocraft-cli-<version>-macos-universal.zip .
spctl --assess --type install -vv photocraft-cli-<version>-macos-universal/photocraft-cli
# ... accepted, source=Notarized Developer ID
```

在 FreeBSD 14（x86_64）上，发布包是按 `/usr/local` 布局的 tarball。先安装运行时库，再解压到那里：

```sh
pkg install libxkbcommon wayland libX11 libXcursor libXrandr libXi libxcb mesa-libs vulkan-loader gtk3 fontconfig freetype2 alsa-lib
tar -xzf photocraft-<version>-freebsd-x86_64.tar.gz --strip-components 1 -C /usr/local
photocraft
```

维护者：[`docs/releasing.md`](docs/releasing.md) 说明了发布如何构建、签名和发布。

> [!IMPORTANT]
> **状态：** PhotoCraft 处于早期 alpha 阶段，我们想坦率说明现状：Photoshop 的大部分功能面已以某种形式存在，但**它还不能替代 Photoshop 用于日常专业工作**。最大的缺口是 AI/生成式功能、15 个缺失工具、排版和专业工作流的深度，以及插件兼容性。每个 Photoshop 菜单项都已接入命令（[`docs/parity-checklist.md`](docs/parity-checklist.md)），但那衡量的是接线，不是行为。诚实的逐维度现状和下一步方向见[路线图](ROADMAP.md)和[对等性评估](docs/target-app-parity.md)。请预料到粗糙之处，并欢迎提交 issue（附上你的操作系统、文档尺寸、图层数量和截图）。你也可以在 [Discord](https://discord.gg/artcraft) 上告诉我们哪里出了问题。

## 社区打包

这些安装包由社区成员维护，而非 PhotoCraft 团队。

在 Gentoo 上，[::snakebyte overlay](https://github.com/switch87/snakebyte-overlay) 将 Linux 发布版打包为 `media-gfx/photocraft-bin`：

```sh
eselect repository add snakebyte git https://github.com/switch87/snakebyte-overlay.git
emaint sync -r snakebyte
echo 'media-gfx/photocraft-bin ~amd64' >> /etc/portage/package.accept_keywords/photocraft
emerge --ask media-gfx/photocraft-bin
```

## 文档

开发者、架构、自动化、格式和安全文档维护在 [PhotoCraft 文档书](book/)中。

要用 Docker 自托管浏览器版本，参见 [Docker Web 托管](packaging/web/README.md#docker)。

## 安全

安全架构、威胁建模、解析器加固、模糊测试和漏洞报告见[安全文档](book/src/security/)和仓库[安全政策](SECURITY.md)。

## 测试语料库

PhotoCraft 用真实文件进行测试：我们自己在 Photoshop 中制作的基准 PSD 位于
[photocraft-corpus](https://github.com/storytold/photocraft-corpus)，外加 psd-tools、ag-psd 和 PngSuite 测试集，均固定版本并经过
sha256 校验。用 `cargo xtask corpus --all` 获取，用
`cargo xtask test-corpus` 运行测试（详见 [docs/development.md](docs/development.md#test-corpora)）。

## 下载

**第一次接触 PhotoCraft？** 请从 [getartcraft.com 的 PhotoCraft 页面](https://getartcraft.com/apps/photocraft)下载。这是最简单的安装方式。

**想要特定构建或格式？** 在 GitHub 上，[最新 release](https://github.com/storytold/photocraft/releases/latest) 列出了下述所有构建，[全部 release](https://github.com/storytold/photocraft/releases) 则有更早的版本及其说明。文件名中的 `<ver>` 是版本号，`SHA256SUMS.txt` 列出每个文件的校验和。

### Windows

| 构建 | 安装包 | 便携版 |
|---|---|---|
| x64（64 位 Intel/AMD） | `photocraft-<ver>-windows-x64.msi` | `photocraft-<ver>-windows-x64-portable.zip` |
| arm64（Snapdragon 及其他 ARM PC） | `photocraft-<ver>-windows-arm64.msi` | `photocraft-<ver>-windows-arm64-portable.zip` |

安装包和可执行文件均已代码签名。

### macOS

| 构建 | 文件 | 说明 |
|---|---|---|
| 应用，通用版（Apple silicon + Intel） | `photocraft-<ver>-macos-universal.dmg` | 已签名并公证 |
| 命令行工具，通用版 | `photocraft-cli-<ver>-macos-universal.zip` | 已签名并公证 |

### Linux

| 格式 | x86_64 | aarch64（ARM64） | 说明 |
|---|---|---|---|
| AppImage | `photocraft-<ver>-linux-x86_64.AppImage` | `photocraft-<ver>-linux-aarch64.AppImage` | 随处运行；通过 [AppImageUpdate](https://github.com/AppImageCommunity/AppImageUpdate) 自更新（`.zsync` 文件） |
| Flatpak | `photocraft-<ver>-linux-x86_64.flatpak` | `photocraft-<ver>-linux-aarch64.flatpak` | 沙箱化；`flatpak install --user <file>` |
| Debian/Ubuntu | `photocraft-<ver>-linux-x86_64.deb` | `photocraft-<ver>-linux-aarch64.deb` | |
| Fedora/RHEL/openSUSE | `photocraft-<ver>-linux-x86_64.rpm` | `photocraft-<ver>-linux-aarch64.rpm` | |
| Tarball | `photocraft-<ver>-linux-x86_64.tar.gz` | `photocraft-<ver>-linux-aarch64.tar.gz` | 解压到任意位置 |

RISC-V（riscv64）：仅 `photocraft-<ver>-linux-riscv64.tar.gz`，基于 Ubuntu 24.04 构建（需要 glibc 2.39+）。

### FreeBSD

| 构建 | 文件 |
|---|---|
| x86_64 | `photocraft-<ver>-freebsd-x86_64.tar.gz` |

### Web（WebAssembly）

| 构建 | 文件 | 说明 |
|---|---|---|
| 静态站点 | `photocraft-web-<ver>.zip` | 在现代浏览器中运行；可托管在任何静态服务器上 |

## Crafting Apps 全家桶

PhotoCraft 是 **Crafting Apps** 之一：来自 [ArtCraft](https://getartcraft.com/) 团队的免费开源创作工具，每一款都用 Rust 从零编写，每一款都能独当一面。

| | 应用 | 用途 | 代码 | 了解更多 |
|:-:|---|---|---|---|
| <img src="https://raw.githubusercontent.com/storytold/photocraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.photocraft.png" alt="" width="32" height="32"> | **PhotoCraft** | 图像编辑：图层、蒙版、文字和真实的 PSD 文件 | [GitHub](https://github.com/storytold/photocraft) | [网站](https://getartcraft.com/apps/photocraft) |
| <img src="https://raw.githubusercontent.com/storytold/vectorcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.vectorcraft.png" alt="" width="32" height="32"> | **VectorCraft** | 矢量插画 | [GitHub](https://github.com/storytold/vectorcraft) | [网站](https://getartcraft.com/apps/vectorcraft) |
| <img src="https://raw.githubusercontent.com/storytold/filmcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.filmcraft.png" alt="" width="32" height="32"> | **FilmCraft** | 视频剪辑、调色和声音 | [GitHub](https://github.com/storytold/filmcraft) | [网站](https://getartcraft.com/apps/filmcraft) |
| <img src="https://raw.githubusercontent.com/storytold/lightcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.lightcraft.png" alt="" width="32" height="32"> | **LightCraft** | 照片库和 raw 显影 | [GitHub](https://github.com/storytold/lightcraft) | [网站](https://getartcraft.com/apps/lightcraft) |
| <img src="https://raw.githubusercontent.com/storytold/pdfcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.pdfcraft.png" alt="" width="32" height="32"> | **PdfCraft** | PDF 阅读、整理和保护 | [GitHub](https://github.com/storytold/pdfcraft) | [网站](https://getartcraft.com/apps/pdfcraft) |
| <img src="https://raw.githubusercontent.com/storytold/effectcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.effectcraft.png" alt="" width="32" height="32"> | **EffectCraft** | 动态图形和视觉特效 | [GitHub](https://github.com/storytold/effectcraft) | [网站](https://getartcraft.com/apps/effectcraft) |
| <img src="https://raw.githubusercontent.com/storytold/designcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.designcraft.png" alt="" width="32" height="32"> | **DesignCraft** | 页面排版和出版 | [GitHub](https://github.com/storytold/designcraft) | [网站](https://getartcraft.com/apps/designcraft) |
| <img src="https://raw.githubusercontent.com/storytold/figcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.figcraft.png" alt="" width="32" height="32"> | **FigCraft** | 界面设计和原型 | [GitHub](https://github.com/storytold/figcraft) | [网站](https://getartcraft.com/apps/figcraft) |
| <img src="https://raw.githubusercontent.com/storytold/wordcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.wordcraft.png" alt="" width="32" height="32"> | **WordCraft** | 文字处理和文档 | [GitHub](https://github.com/storytold/wordcraft) | [网站](https://getartcraft.com/apps/wordcraft) |
| <img src="https://raw.githubusercontent.com/storytold/soundcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.soundcraft.png" alt="" width="32" height="32"> | **SoundCraft** | 音频录制和编辑 | [GitHub](https://github.com/storytold/soundcraft) | [网站](https://getartcraft.com/apps/soundcraft) |
| <img src="https://raw.githubusercontent.com/storytold/deckcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.deckcraft.png" alt="" width="32" height="32"> | **DeckCraft** | 演示文稿和幻灯片 | [GitHub](https://github.com/storytold/deckcraft) | [网站](https://getartcraft.com/apps/deckcraft) |
| <img src="https://raw.githubusercontent.com/storytold/cadcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.cadcraft.png" alt="" width="32" height="32"> | **CADCraft** | 计算机辅助设计 | [GitHub](https://github.com/storytold/cadcraft) | [网站](https://getartcraft.com/apps/cadcraft) |
| <img src="https://raw.githubusercontent.com/storytold/gridcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.gridcraft.png" alt="" width="32" height="32"> | **GridCraft** | 电子表格和数据 | [GitHub](https://github.com/storytold/gridcraft) | [网站](https://getartcraft.com/apps/gridcraft) |
| <img src="https://raw.githubusercontent.com/storytold/texturecraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.texturecraft.png" alt="" width="32" height="32"> | **TextureCraft** | 3D 纹理绘制 | [GitHub](https://github.com/storytold/texturecraft) | [网站](https://getartcraft.com/apps/texturecraft) |
| <img src="https://raw.githubusercontent.com/storytold/craft-anything/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.craft-anything.png" alt="" width="32" height="32"> | **Craft Anything** | 看懂任何软件的工作原理，然后构建你自己的 | [GitHub](https://github.com/storytold/craft-anything) | [网站](https://getartcraft.com/apps/craft-anything) |
| <img src="https://raw.githubusercontent.com/tangency-labs/solidcraft/main/assets/icons/app/linux/exports/hicolor/64x64/apps/io.tangencylabs.solidcraft.png" alt="" width="32" height="32"> | **SolidCraft** | Tangency Labs 的参数化 3D CAD（社区应用） | [GitHub](https://github.com/tangency-labs/solidcraft) | |

还有 [**ArtCraft**](https://getartcraft.com/) 本身，我们为追求真正掌控力的艺术家打造的 AI 图像与视频工作室。

Crafting Apps 遵循同样的约定：净室实现、纯 Rust、在 macOS、Windows 和 Linux 上原生运行、通过 WebAssembly 进入浏览器，并且完全可由 agent 驱动。

<br>

<p align="center">
  <a href="https://discord.gg/artcraft"><img alt="Join the ArtCraft community on Discord" src="https://img.shields.io/badge/Join%20us%20on%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="40"></a>
</p>

<h3 align="center">来和我们一起创作</h3>

<p align="center">
  我们的 Discord 是各类艺术家聚集的地方：绘画的、摄影的、画画的、剪片子的、
  排字的，还有仍在摸索自己喜欢创作什么的人。分享你正在做的作品、
  寻求帮助、告诉我们哪里坏了，或告诉我们你希望这些工具还能做什么。
  无论你的媒介是什么、入行多久，这里都欢迎你。
</p>

<p align="center">
  <a href="https://discord.gg/artcraft"><b>discord.gg/artcraft</b></a> ·
  <a href="https://getartcraft.com/">getartcraft.com</a> ·
  <a href="https://getartcraft.com/apps">The Crafting Apps</a> ·
  <a href="https://getartcraft.com/apps/photocraft">PhotoCraft</a>
</p>

---

## 许可证与致谢

PhotoCraft 以 [MIT](LICENSE-MIT) 或 [Apache-2.0](LICENSE-APACHE) 双重许可，任你选择。
版权所有 (c) 2026 ArtCraft 团队及 PhotoCraft 贡献者。必需的声明见 [NOTICE](NOTICE)。

随附的字体、图标、图像和其他资源保留各自的开源许可证；每一项都在
[ATTRIBUTION.md](ATTRIBUTION.md) 中列出了作者、来源和许可证。

展示的所有艺术作品均属公有领域（Wikimedia Commons、NASA、美国国家档案馆）；来源列于 [`docs/images/SOURCES.md`](docs/images/SOURCES.md)。

[`docs/brand/`](docs/brand/) 中的 ArtCraft 名称、文字标志和徽标是 ArtCraft 团队的商标，
不在本许可证覆盖范围内。它们只能以未修改的形式、且仅作为本仓库和 PhotoCraft 的一部分使用，
遵循 [`docs/brand/LICENSE-brand.txt`](docs/brand/LICENSE-brand.txt)。
Fork 和修改版本必须移除它们。

<sub>Adobe、Photoshop、Illustrator、Premiere Pro、Lightroom、Acrobat、After Effects 和 InDesign 是 Adobe Inc. 在美国和/或其他国家的商标或注册商标。PhotoCraft 是一个独立的开源项目，与 Adobe Inc. 无关联、未获其赞助或认可；这些名称仅用于描述其兼容的工作流。</sub>

<p align="center">
  <a href="https://getartcraft.com/"><img alt="ArtCraft" src="docs/brand/artcraft-mark.svg" width="28"></a><br>
  <sub>由 <a href="https://getartcraft.com/">ArtCraft</a> 团队和社区打造。</sub>
</p>

## Star 历史

[![Star History Chart](https://api.star-history.com/svg?repos=storytold/photocraft&type=Date&legend=top-left)](https://www.star-history.com/?repos=storytold%2Fphotocraft&type=date&legend=top-left)