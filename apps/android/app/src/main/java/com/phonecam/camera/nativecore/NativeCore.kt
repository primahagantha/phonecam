package com.phonecam.camera.nativecore

import java.io.File

/**
 * JNI bridge into the shared Rust `phonecam-android-core` library.
 *
 * On Android devices this loads `libphonecam_android_core.so` from `jniLibs`
 * via [System.loadLibrary]. On desktop JVMs (used by Gradle unit tests), it
 * resolves `phonecam.native.lib` or searches the Cargo `target/release` and
 * `target/debug` directories so tests execute the real compiled Rust code.
 */
object NativeCore {

    init {
        loadNativeLibrary()
    }

    private fun loadNativeLibrary() {
        val explicit = System.getProperty("phonecam.native.lib")?.trim()
        if (!explicit.isNullOrEmpty()) {
            val file = File(explicit)
            if (file.isFile) {
                System.load(file.canonicalFile.absolutePath)
                return
            }
        }

        val vmName = System.getProperty("java.vm.name").orEmpty()
        val vendor = System.getProperty("java.vendor").orEmpty()
        val isAndroid = vmName.contains("Dalvik", ignoreCase = true) ||
            vendor.contains("Android", ignoreCase = true)
        if (isAndroid) {
            System.loadLibrary("phonecam_android_core")
            return
        }

        val osName = System.getProperty("os.name").orEmpty().lowercase()
        val libName = when {
            osName.contains("win") -> "phonecam_android_core.dll"
            osName.contains("mac") -> "libphonecam_android_core.dylib"
            else -> "libphonecam_android_core.so"
        }

        val candidates = listOf(
            File("../../target/release/$libName"),
            File("../../target/debug/$libName"),
            File("../../../target/release/$libName"),
            File("../../../target/debug/$libName"),
            File("target/release/$libName"),
            File("target/debug/$libName"),
        )
        val found = candidates.firstOrNull { it.isFile }
        if (found != null) {
            System.load(found.canonicalFile.absolutePath)
        } else {
            System.loadLibrary("phonecam_android_core")
        }
    }

    external fun nativeVersion(): String

    external fun randomHex(byteLen: Int): String

    external fun secretFingerprint(secretHex: String): String

    external fun deviceSecretAuth(
        deviceId: String,
        secretHex: String,
        serverNonce: ByteArray,
        clientNonce: Long,
    ): String

    external fun pairRequestAuth(
        deviceId: String,
        secretHex: String?,
        serverNonce: ByteArray,
        clientNonce: Long,
    ): String

    external fun decodeServerGreeting(bytes: ByteArray): ByteArray

    external fun encodeClientHandshake(
        role: Int,
        authMode: Int,
        clientNonce: Long,
        authBytes: ByteArray,
    ): ByteArray

    external fun decodeServerResponseHeader(header24: ByteArray): IntArray

    external fun encodeVideoPacketHeader(
        frameId: Long,
        ptsUs: Long,
        flags: Int,
        codec: Int,
        payloadLen: Int,
    ): ByteArray

    external fun encodeVideoPacket(
        frameId: Long,
        ptsUs: Long,
        flags: Int,
        codec: Int,
        payload: ByteArray,
        offset: Int,
        length: Int,
    ): ByteArray

    external fun decodeVideoPacketHeader(header24: ByteArray): LongArray

    external fun validateQrPayload(raw: String): String?

    external fun parseDiscoveryBeacon(jsonBytes: ByteArray): String?

    external fun adaptiveQualityCreate(
        enabled: Boolean,
        initRes: String,
        initFps: Int,
    ): Long

    external fun adaptiveQualitySetEnabled(handle: Long, enabled: Boolean)

    external fun adaptiveQualityConfirm(handle: Long, res: String, fps: Int)

    external fun adaptiveQualityObserve(
        handle: Long,
        nowMs: Long,
        curRes: String,
        curFps: Int,
        maxRes: String,
        maxFps: Int,
        droppedPct: Double,
        rttUs: Long,
        thermal: String,
        decodeStruggling: Boolean,
    ): String

    external fun adaptiveQualityDestroy(handle: Long)
}
