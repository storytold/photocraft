# ComfyUI workflow fixtures

These API graph fixtures document the audited workflow shapes. Replace input
filenames, prompts, output prefixes and other placeholders before execution.
They contain no personal endpoint, credential or image reference.

- Text-to-image and reference-image editing with the installed Qwen profile.
- Selection-limited generation and final masked composition.
- Canvas expansion using a padded source and an editable edge mask.
- BiRefNet foreground masks.
- MoGe depth visualization as research evidence, not a shipped metric-depth tool.

The application's validated runtime graphs live in `crates/ai/src/native/`.
Those graphs are instantiated with unique owned upload/output identifiers and
checked against server nodes/model choices before submission. Research graphs
are not an unrestricted user-supplied execution surface.

See [capability audit](../comfyui-audit-2026-10-08.md) and
[AI usage](../ai-usage.md) for tested behavior and limits.
