package com.phonecam.camera.protocol

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.descriptors.PrimitiveKind
import kotlinx.serialization.descriptors.PrimitiveSerialDescriptor
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.intOrNull

/**
 * Handles wire serialization for FPS which can arrive as an integer (e.g. 30, 60)
 * or as the string "auto" (§6.1).
 */
object FpsSerializer : KSerializer<Int> {
    override val descriptor: SerialDescriptor =
        PrimitiveSerialDescriptor("Fps", PrimitiveKind.INT)

    override fun serialize(encoder: Encoder, value: Int) {
        if (value <= 0) {
            encoder.encodeString("auto")
        } else {
            encoder.encodeInt(value)
        }
    }

    override fun deserialize(decoder: Decoder): Int {
        val jsonDecoder = decoder as? JsonDecoder
        if (jsonDecoder != null) {
            val element = jsonDecoder.decodeJsonElement()
            return when {
                element is JsonPrimitive && element.isString -> {
                    if (element.content.equals("auto", ignoreCase = true)) -1
                    else element.content.toIntOrNull() ?: -1
                }
                element is JsonPrimitive -> element.intOrNull ?: -1
                else -> -1
            }
        }
        return runCatching { decoder.decodeInt() }.getOrElse { -1 }
    }
}

/**
 * Control-channel messages, matching `docs/protocol.md` §6.
 *
 * Field names are the wire contract: renaming a property is a protocol change.
 */
@Serializable
sealed class PhoneMessage {
    @Serializable
    @SerialName("hello")
    data class Hello(
        val protocol: Int,
        val deviceId: String,
        val deviceName: String,
        val model: String,
        val androidVersion: Int,
        val appVersion: String,
    ) : PhoneMessage()

    @Serializable
    @SerialName("capabilities")
    data class Capabilities(
        val device: DeviceInfo,
        val cameras: List<CameraInfo>,
    ) : PhoneMessage()

    @Serializable
    @SerialName("state")
    data class State(
        val streaming: Boolean,
        val camera: String,
        val resolution: String,
        val fps: Int,
        val actualResolution: String,
        val actualFps: Int,
        val bitrate: Int,
        val mirror: Boolean,
        val rotation: Int,
        val torch: Boolean,
        val zoom: Float,
        val battery: Int,
        val charging: Boolean,
        val thermal: String,
    ) : PhoneMessage()

    @Serializable
    @SerialName("stats")
    data class Stats(
        val fps: Double,
        val bitrate: Int,
        val dropped: Double,
        val encodedFrames: Long,
        val rttUs: Long,
        val thermal: String,
    ) : PhoneMessage()

    @Serializable
    @SerialName("pong")
    data class Pong(val clientNonce: Long) : PhoneMessage()

    @Serializable
    @SerialName("pair_token")
    data class PairToken(val token: String) : PhoneMessage()

    @Serializable
    @SerialName("error")
    data class Error(val code: String, val message: String) : PhoneMessage()
}

@Serializable
data class DeviceInfo(
    val model: String,
    val androidVersion: Int,
    val appVersion: String,
)

@Serializable
data class CameraInfo(
    val id: String,
    val facing: String,
    val resolutions: List<String>,
    val fps: List<Int>,
    val torch: Boolean,
    val focus: Boolean,
    val zoom: Boolean,
    val exposure: Boolean,
    val whiteBalance: Boolean,
    val minZoom: Float,
    val maxZoom: Float,
    val orientationDegrees: Int,
)

/** Desktop → phone commands. Unknown types are ignored, never fatal. */
@Serializable
sealed class DesktopMessage {
    @Serializable
    @SerialName("welcome")
    data class Welcome(
        val protocol: Int,
        val serverVersion: String,
        val serverName: String,
    ) : DesktopMessage()

    @Serializable
    @SerialName("set_camera")
    data class SetCamera(val camera: String) : DesktopMessage()

    @Serializable
    @SerialName("set_video")
    data class SetVideo(
        val resolution: String,
        @Serializable(with = FpsSerializer::class)
        val fps: Int = -1,
        val bitrate: Int,
    ) : DesktopMessage()

    @Serializable
    @SerialName("set_image")
    data class SetImage(
        val mirror: Boolean,
        val rotation: Int,
        val zoom: Float,
    ) : DesktopMessage()

    @Serializable
    @SerialName("set_torch")
    data class SetTorch(val enabled: Boolean) : DesktopMessage()

    @Serializable
    @SerialName("set_screen")
    data class SetScreen(val mode: String) : DesktopMessage()

    @Serializable
    @SerialName("set_quality_mode")
    data class SetQualityMode(val mode: String) : DesktopMessage()

    @Serializable
    @SerialName("start_stream")
    data object StartStream : DesktopMessage()

    @Serializable
    @SerialName("stop_stream")
    data object StopStream : DesktopMessage()

    @Serializable
    @SerialName("ping")
    data class Ping(val serverNonce: Long) : DesktopMessage()

    @Serializable
    @SerialName("pair_approved")
    data class PairApproved(val token: String) : DesktopMessage()

    @Serializable
    @SerialName("pair_denied")
    data class PairDenied(val reason: String) : DesktopMessage()

    @Serializable
    @SerialName("error")
    data class Error(val code: String, val message: String) : DesktopMessage()
}

/**
 * The single JSON codec used on the wire.
 *
 * `classDiscriminator = "t"` matches the TS `{ "t": ... }` envelope.
 * `ignoreUnknownKeys` implements forward compatibility: a newer desktop may add
 * fields, and this app must keep working rather than throwing.
 */
object ControlCodec {
    val json: Json = Json {
        classDiscriminator = "t"
        ignoreUnknownKeys = true
        encodeDefaults = true
        explicitNulls = false
        isLenient = false
    }

    fun encode(message: PhoneMessage): ByteArray =
        json.encodeToString(PhoneMessage.serializer(), message).toByteArray(Charsets.UTF_8)

    fun encodeDiscovery(value: JsonObject): ByteArray =
        value.toString().toByteArray(Charsets.UTF_8)

    /** Returns `null` for unknown message types so callers can log and skip. */
    fun decodeDesktop(payload: ByteArray): DesktopMessage? =
        runCatching {
            json.decodeFromString(DesktopMessage.serializer(), String(payload, Charsets.UTF_8))
        }.getOrNull()
}
