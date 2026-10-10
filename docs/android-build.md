# Compilando PhotoCraft para Android no PC

Guia para a branch **android-native** do fork maxold1985/photocraft.
O aplicativo e nativo: Rust/egui + Vulkan, empacotado pelo Android Gradle Plugin.
**O Android Studio nao compila Rust automaticamente.** Primeiro compile a
biblioteca .so com Cargo, depois gere o APK com Gradle.

## Requisitos

- Windows 10/11 x64, Git, rustup e Rust stable.
- Android Studio, JDK **17** e Gradle **8.11.1** no PATH.
- SDK Android **35**, Build Tools **35.0.0**, NDK **27.2.12479018**.
- Dispositivo ARM64 (arm64-v8a), Android API 26 ou superior e Vulkan.

No Android Studio, instale as versoes especificadas em Settings >
Languages & Frameworks > Android SDK (em SDK Tools, ative Show Package
Details para selecionar a versao exata do NDK).

O projeto usa Android Gradle Plugin **8.9.2**. Ainda nao inclui
gradlew/gradlew.bat: instale Gradle 8.11.1 separadamente ou configure um
wrapper compativel antes de sincronizar no Android Studio.

## 1. Baixar o projeto (PowerShell)

~~~powershell
git clone --branch android-native https://github.com/maxold1985/photocraft.git
cd photocraft
rustup target add aarch64-linux-android
java -version
gradle --version
~~~

Use Java 17 e confira a versao 8.11.1 do Gradle.

## 2. Compilar Rust ARM64 (PowerShell, na raiz do repositorio)

~~~powershell
$env:ANDROID_HOME = "$env:LOCALAPPDATA\Android\Sdk"
$env:ANDROID_SDK_ROOT = $env:ANDROID_HOME
$ndk = Join-Path $env:ANDROID_HOME "ndk\27.2.12479018"
$bin = Join-Path $ndk "toolchains\llvm\prebuilt\windows-x86_64\bin"

$cc = Join-Path $bin "aarch64-linux-android26-clang.cmd"
$cxx = Join-Path $bin "aarch64-linux-android26-clang++.cmd"
if (!(Test-Path $cc)) { throw "NDK compiler not found: $cc" }
if (!(Test-Path $cxx)) { throw "NDK C++ compiler not found: $cxx" }

$env:CC_aarch64_linux_android = $cc
$env:CXX_aarch64_linux_android = $cxx
$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = $cc
$env:AR_aarch64_linux_android = Join-Path $bin "llvm-ar.exe"

cargo build --release --target aarch64-linux-android -p photocraft-android
if ($LASTEXITCODE -ne 0) { throw "Cargo build failed" }

$dst = "apps\photocraft-android\android\src\main\jniLibs\arm64-v8a"
New-Item -ItemType Directory -Force $dst | Out-Null
Copy-Item "target\aarch64-linux-android\release\libphotocraft_android.so" $dst
~~~

A biblioteca deve existir em
apps/photocraft-android/android/src/main/jniLibs/arm64-v8a/libphotocraft_android.so.

Se seu NDK fornecer compiladores com outra extensao, ajuste os caminhos
de CC, CXX e LINKER para os executaveis reais em bin.

## 3. Compilar APK no console (PowerShell)

~~~powershell
cd apps\photocraft-android\android
gradle --no-daemon :app:assembleDebug
~~~

O arquivo resultante, relativo a raiz do repositorio, e:

~~~text
apps/photocraft-android/android/app/build/outputs/apk/debug/app-debug.apk
~~~

**Nao execute Gradle antes de copiar o .so:** o APK pode ficar sem
a biblioteca Rust e falhar ao iniciar.

## 4. Compilar e instalar usando Android Studio

1. Android Studio > Open > selecione a pasta
   photocraft/apps/photocraft-android/android (nao a raiz Rust).
2. Configure JDK 17 e Gradle 8.11.1. Como nao ha wrapper versionado,
   use a instalacao local do Gradle ou gere um wrapper compativel.
3. Sincronize o projeto e confirme SDK 35 e NDK 27.2.12479018.
4. Compile e copie a biblioteca Rust usando o passo 2.
5. Use Build > Build APK(s) ou o comando Gradle do passo 3.
6. Ative Depuracao USB no Android, conecte o dispositivo e instale o APK.

Sempre que modificar codigo Rust, repita Cargo build, copie o .so e
gere outro APK: o Android Studio nao faz essas etapas automaticamente.

## 5. Instalar e diagnosticar pelo ADB (PowerShell)

A partir da pasta apps/photocraft-android/android:

~~~powershell
adb devices
adb install -r "app\build\outputs\apk\debug\app-debug.apk"
adb logcat -c
adb logcat -v time -s AndroidRuntime libc DEBUG RustStdoutStderr
~~~

Para salvar os logs apos uma falha:

~~~powershell
adb logcat -d -v time > photocraft-android-log.txt
~~~

Se adb nao estiver no PATH, use o executavel em Android/Sdk/platform-tools.

## 6. Alternativa: GitHub Actions

O workflow .github/workflows/android-native.yml instala as dependencias
no Linux, compila a biblioteca Rust ARM64 e monta o APK automaticamente.

Abra Actions > PhotoCraft Android ARM64 > Run workflow, escolha
android-native e baixe o artefato PhotoCraft-Android-arm64-debug.

## Erros comuns

| Problema | Solucao |
| --- | --- |
| Linker do Rust nao encontrado | Verifique NDK e CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER |
| SDK location not found | Configure ANDROID_HOME ou local.properties com sdk.dir |
| Versao Gradle incompatível | Use Gradle 8.11.1, AGP 8.9.2 e JDK 17 |
| UnsatisfiedLinkError | Verifique libphotocraft_android.so em jniLibs/arm64-v8a e reconstrua |
| ADB nao encontra o tablet | Ative Depuracao USB e autorize o PC |
| Falha ao criar imagens grandes | Atualize android-native; o canvas CPU e o fallback atual |

A interface egui permanece em Vulkan; o canvas de documentos usa CPU
temporariamente no Android para evitar a falha do compositor GPU.
