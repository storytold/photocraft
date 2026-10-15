@echo off
rem Deploy the neural-matting runtime: ONNX Runtime (DirectML) DLLs next to the
rem executables, plus the model directory that holds the cutout weights.
rem
rem ONNX Runtime ships as the NuGet package Microsoft.ML.OnnxRuntime.DirectML;
rem the loader (ort, load-dynamic) picks the DLLs up from the executable's
rem directory at run time — nothing is linked at build time.
rem
rem Model weights are NOT distributed here. The user supplies a BiRefNet ONNX
rem export (MIT licence) from the model authors' page
rem https://huggingface.co/ZhengPeng7/BiRefNet and drops it in as
rem   PhotoCraftData\models\mattingirefnet-general.onnx
rem optionally adding birefnet-hr.onnx for the sharper 2048 variant.
setlocal
cd /d "%~dp0.."

set "ORT_SRC=%~1"
if "%ORT_SRC%"=="" (
  echo Usage: deploy_matting.bat ^<path to unpacked Microsoft.ML.OnnxRuntime.DirectML package or its runtimes\win-x64
ative folder^>
  echo.
  echo Unpack the .nupkg with:  tar -xf Microsoft.ML.OnnxRuntime.DirectML.x.y.z.nupkg -C onnxruntime-dml
  echo then pass: onnxruntime-dmluntimes\win-x64
ative
  exit /b 2
)

set "SRC=%ORT_SRC%"
if exist "%SRC%untimes\win-x64
ative\onnxruntime.dll" set "SRC=%SRC%untimes\win-x64
ative"

for %%f in (onnxruntime.dll DirectML.dll onnxruntime_providers_shared.dll) do (
  if not exist "%SRC%\%%f" (
    echo ERROR: %%f not found under "%SRC%"
    exit /b 1
  )
  copy /y "%SRC%\%%f" ".\%%f" >nul
  echo installed %%f
)

if not exist "PhotoCraftData\models\matting" mkdir "PhotoCraftData\models\matting"
if not exist "PhotoCraftData\models\mattingirefnet-general.onnx" (
  echo.
  echo NOTE: place birefnet-general.onnx into PhotoCraftData\models\matting  ^(download link above^).
) else (
  echo model present: PhotoCraftData\models\mattingirefnet-general.onnx
)
echo done.
endlocal
