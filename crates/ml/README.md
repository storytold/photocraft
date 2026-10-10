# photocraft-ml

Native local BiRefNet General Lite inference through ONNX Runtime's CPU provider.
The model uses a 1024 × 1024 ImageNet-normalized RGB input, sigmoid and min/max
normalization of the logits, and Lanczos resizing of a continuous foreground matte,
matching rembg's BiRefNet session used in CanvaSmith. Constant or nonfinite output
returns an error rather than destroying a layer.

Run `scripts/setup-background-removal.sh` from a source checkout. The 214 MiB model
is checksum-verified and ignored by Git. Set `PHOTOCRAFT_BACKGROUND_MODEL` to an
absolute ONNX path to use an installed model. For packaged builds place it in
`models/` beside the executable, or `Contents/Resources/models/` on macOS, alongside
`LICENSE-BiRefNet.txt`. ONNX Runtime is linked by the native Cargo dependency.
Browser builds return an explicit desktop-only error and have no ONNX dependency.

`layer.removeBackground {"method":"ai"}` analyzes only the requested raster layer
and creates a non-destructive, undoable layer mask. The existing subject detector
remains available with `"method":"subject"` (the command's compatibility default).

Sources: https://github.com/danielgatis/rembg/tree/main/rembg/sessions,
https://github.com/ZhengPeng7/BiRefNet. Model license: MIT, ZhengPeng (2024).
