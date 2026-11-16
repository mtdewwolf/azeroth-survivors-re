import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dir, "..");
const sdk = process.env.ANDROID_HOME ?? process.env.ANDROID_SDK_ROOT;
const ndk = process.env.ANDROID_NDK_HOME;
if (!sdk || !ndk) throw new Error("Set ANDROID_HOME and ANDROID_NDK_HOME.");

const tools = process.env.ANDROID_BUILD_TOOLS ?? join(sdk, "build-tools", "35.0.0");
const platform = process.env.ANDROID_PLATFORM_JAR ?? join(sdk, "platforms", "android-35", "android.jar");
const host = process.env.ANDROID_NDK_HOST_TAG ?? (process.platform === "darwin" ? "darwin-x86_64" : "linux-x86_64");
const llvm = join(ndk, "toolchains", "llvm", "prebuilt", host, "bin");
const state = join(root, "android", ".build");
const stage = join(state, "stage");
const classes = join(state, "classes");
const output = resolve(process.argv[2] ?? join(state, "Azeroth-Survivors-Progression.apk"));
const target = "aarch64-linux-android";
const compiler = join(llvm, target + "26");

function run(command: string[], cwd = root, extraEnv: Record<string, string> = {}) {
    const result = Bun.spawnSync(command, { cwd, env: { ...process.env, ...extraEnv }, stdout: "inherit", stderr: "inherit" });
    if (!result.success) throw new Error(`${command[0]} exited with status ${result.exitCode}`);
}

function files(dir: string, suffix: string): string[] {
    return readdirSync(dir, { recursive: true, encoding: "utf8" })
        .filter(name => name.endsWith(suffix)).map(name => join(dir, name));
}

if (process.env.ANDROID_SKIP_RUST_BUILD !== "1") {
    run(["cargo", "build", "--locked", "--release", "--target", target, "-p", "azeroth-android"], root, {
        CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER: compiler + "-clang",
        CC_aarch64_linux_android: compiler + "-clang",
        CXX_aarch64_linux_android: compiler + "-clang++",
        AR_aarch64_linux_android: join(llvm, "llvm-ar"),
        CARGO_PROFILE_RELEASE_DEBUG: "0",
        CARGO_PROFILE_RELEASE_STRIP: "symbols",
        CARGO_PROFILE_RELEASE_CODEGEN_UNITS: "32",
    });
}

const library = "libazeroth_android.so";
const native = join(resolve(root, process.env.CARGO_TARGET_DIR ?? "target"), target, "release", library);
if (!existsSync(native)) throw new Error(`Missing Android native library at ${native}`);
rmSync(stage, { recursive: true, force: true });
rmSync(classes, { recursive: true, force: true });
mkdirSync(classes, { recursive: true });
const abi = join(stage, "lib", "arm64-v8a");
const licenses = join(stage, "assets", "licenses");
mkdirSync(abi, { recursive: true });
mkdirSync(licenses, { recursive: true });
copyFileSync(native, join(abi, library));
for (const [source, label] of [
    ["LICENSE-APACHE", "BENILLA-APACHE"], ["LICENSE-MIT", "BENILLA-MIT"],
    ["third_party/kira/LICENSE-APACHE", "KIRA-APACHE"],
    ["third_party/kira/LICENSE-MIT", "KIRA-MIT"], ["third_party/lua-src/LICENSE", "LUA"],
]) copyFileSync(join(root, source), join(licenses, label));

const ecj = process.env.ANDROID_ECJ_JAR;
const javaCompiler = ecj ? ["java", "-jar", ecj, "-proc:none", "-source", "1.8", "-target", "1.8"]
    : ["javac", "--release", "8"];
run([...javaCompiler, "-classpath", platform, "-d", classes, ...files(join(root, "android", "app", "src"), ".java")]);
run([join(tools, "d8"), "--release", "--min-api", "26", "--lib", platform, "--output", stage, ...files(classes, ".class")]);
const unsigned = join(state, "unsigned.apk");
const aligned = join(state, "aligned.apk");
const resources = join(state, "resources.zip");
rmSync(unsigned, { force: true });
run([join(tools, "aapt2"), "compile", "--dir", join(root, "android", "app", "res"), "-o", resources]);
run([join(tools, "aapt2"), "link", "-I", platform, "--manifest", join(root, "android", "app", "AndroidManifest.xml"), "-o", unsigned, resources]);
run(["zip", "-q", "-9", "-r", unsigned, "classes.dex", "lib", "assets"], stage);
run([join(tools, "zipalign"), "-P", "16", "-f", "4", unsigned, aligned]);

const keystore = resolve(process.env.ANDROID_KEYSTORE ?? join(state, "android-local.p12"));
const alias = process.env.ANDROID_KEY_ALIAS ?? "azeroth-local";
const signEnv = { AZEROTH_APK_PASSWORD: process.env.ANDROID_KEYSTORE_PASSWORD ?? "android" };
if (!existsSync(keystore)) {
    if (process.env.ANDROID_KEYSTORE) throw new Error("The supplied keystore does not exist.");
    run(["keytool", "-genkeypair", "-keystore", keystore, "-storetype", "PKCS12", "-storepass:env",
        "AZEROTH_APK_PASSWORD", "-alias", alias, "-keyalg", "RSA", "-keysize", "3072", "-validity",
        "10000", "-dname", "CN=Azeroth Survivors Progression Sideload"], root, signEnv);
}
const signer = join(tools, "apksigner");
run([signer, "sign", "--ks", keystore, "--ks-key-alias", alias, "--ks-pass",
    "env:AZEROTH_APK_PASSWORD", "--v4-signing-enabled", "false", "--out", output, aligned], root, signEnv);
run([signer, "verify", "--verbose", output]);
run([join(tools, "zipalign"), "-c", "-P", "16", "4", output]);
console.log(output);
