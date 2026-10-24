# Windows guardians

> **Last updated:** 2026-10-22 · **Change:** major (first version: the Windows-specific guards in the code, and why each exists)

Windows imposes rules that macOS and Linux don't: security features that block writes, antivirus
scanners that hold files open, a desktop that isolates processes by privilege, DLLs found through
a search path, drivers with known crashes. Users meet them as "PhotoCraft won't save" or
"dropping a file does nothing", with no hint of the cause.

A **guardian** is the code that meets one of these rules on purpose. It **detects** the
situation, then **works around it** where it can, and otherwise **tells the user** what Windows
did and what to do instead. Each one follows [AGENTS.md](../AGENTS.md) › *Never crash*: an error
or notice the user can act on, never a panic. This page lists them so a contributor knows why
the code looks the way it does before "simplifying" it.

## Files and saving

| Guardian | The Windows rule | What PhotoCraft does | Code |
|---|---|---|---|
| Retried replace | Antivirus scanners and the search indexer briefly hold a just-written file open, so replacing it fails with `PermissionDenied` | Retries the rename with backoff (about 1.3 s in total) before reporting an error | `crates/format/src/atomic.rs` (`RenameRetry`) |
| `ReplaceFileW` saves | A plain rename over a file drops its ACL, its alternate streams (the Mark of the Web, `Zone.Identifier`) and its creation time | Replaces the existing file with `ReplaceFileW` and a backup name, falling back to a rename only when that fails | `crates/format/src/atomic/meta.rs` (#7094, #7096) |
| Device names | `CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9` (and `COM¹`–`³`, `LPT¹`–`³`) name devices, with any extension: a file or folder of that name opens the device | Generated file and folder names get a `_` before the first dot (`nul` → `nul_`) | `crates/engine/src/file_cmds.rs` (`sanitize`, `is_windows_device_name`) |
| Same file | Paths alone can't tell whether two names are one file (hard links, `8.3` names, case) | Compares the volume serial number and file index, as Unix compares device and inode | `crates/engine/src/file_cmds.rs` |

## Graphics

| Guardian | The Windows rule | What PhotoCraft does | Code |
|---|---|---|---|
| Shader compiler by full path | wgpu loads `dxcompiler.dll` by name, and the DLL search reaches the current directory and `PATH`, where other programs ship incompatible builds | Uses a `dxcompiler.dll` beside the executable by its full path, otherwise FXC (`d3dcompiler_47.dll`, part of Windows) | `apps/photocraft/src/gpu_startup.rs` (`dx12_compiler`, #712) |
| No Vulkan on Intel | Creating a Vulkan instance loads the Intel Vulkan driver, which crashes on some machines | On Intel, `auto` uses DirectX 12 with GL as the last resort and never loads Vulkan | `gpu_startup.rs` (`windows_auto_backends`) |
| GPU start-up marker | A driver can crash the process while the device is being created | A locked marker file before device creation; the next launch that finds it starts in CPU mode | `gpu_startup.rs` |
| Software graphics notice | Without a usable adapter Windows offers WARP, its software renderer, which is far slower | Says so at start-up ("using software graphics") | `apps/photocraft/src/main.rs` |

## Start-up and system

| Guardian | The Windows rule | What PhotoCraft does | Code |
|---|---|---|---|
| Delay-loaded PDF component | Stripped Windows images (NTLite, some N and LTSC editions) lack `windows.data.pdf.dll`; a load-time import stops the executable from starting at all | Loads it on first use; printing checks for it and reports an error | `apps/photocraft/build.rs` (#5884) |
| Busy clipboard | Another program can hold the system clipboard open | Bounded retries, then "clipboard is busy" instead of hanging | `apps/photocraft/src/services.rs` |
| **Elevated file drops** | User Interface Privilege Isolation (UIPI) drops the drag-and-drop messages that a non-elevated app such as File Explorer sends to an elevated window, silently on both sides, so "Run as administrator" makes every drop do nothing | Detects at start-up that the process token is the full half of a split UAC token (`TokenElevationTypeFull`) and shows a dismissible notice: use File › Open, paste a copied file, or start PhotoCraft without Run as administrator. With UAC off every process of an administrator, Explorer included, runs at the same level and drops work, so `TokenIsElevated` (true there) would be a false alarm | `apps/photocraft/src/elevation.rs`, `crates/ui-egui/src/notices.rs` (`elevated_file_drop_guidance`) |

## Adding a guardian

- Detect the condition. Don't infer it from a symptom when Windows can be asked directly.
- Prefer working around the rule. When that isn't possible, tell the user what Windows did and
  what to do instead, in their language (notices take English templates translated when drawn).
- A notice that will keep appearing gets a `dismiss_pref`, so a user who has read it can turn it off.
- Add a test for the detection or its consequences, and a row here with the rule it answers.
