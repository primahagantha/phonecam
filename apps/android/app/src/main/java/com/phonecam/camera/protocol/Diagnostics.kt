package com.phonecam.camera.protocol

import android.content.Context
import android.os.Build
import android.os.PowerManager
import androidx.core.content.ContextCompat
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

enum class LogLevel(val order: Int, val label: String) {
    ERROR(0, "ERROR"),
    WARN(1, "WARN"),
    INFO(2, "INFO"),
    DEBUG(3, "DEBUG"),
    TRACE(4, "TRACE"),
    ;

    companion object {
        fun parse(value: String?): LogLevel =
            entries.firstOrNull { it.label.equals(value, ignoreCase = true) } ?: INFO
    }
}

/**
 * In-memory ring buffer plus an on-disk file, mirroring PRD §63.
 *
 * Two rules are enforced here rather than left to call sites:
 *  - pairing tokens and device secrets are redacted before they can reach a log;
 *  - video payload bytes are never logged, only lengths and timings.
 */
object Log {
    private const val MAX_MEMORY_LINES = 500
    private const val MAX_FILE_BYTES = 512L * 1024L
    private const val TAG = "PhoneCam"

    private val lines = ArrayDeque<String>(MAX_MEMORY_LINES)
    private val timestampFormat = SimpleDateFormat("HH:mm:ss.SSS", Locale.US)

    @Volatile
    private var level: LogLevel = LogLevel.INFO

    @Volatile
    private var logFile: File? = null

    @Volatile
    private var listener: ((String) -> Unit)? = null

    fun configure(context: Context, minLevel: LogLevel) {
        level = minLevel
        val dir = File(context.filesDir, "logs").apply { mkdirs() }
        logFile = File(dir, "phonecam.log")
    }

    fun setLevel(minLevel: LogLevel) {
        level = minLevel
    }

    fun setListener(callback: ((String) -> Unit)?) {
        listener = callback
    }

    fun e(message: String, throwable: Throwable? = null) = write(LogLevel.ERROR, message, throwable)
    fun w(message: String, throwable: Throwable? = null) = write(LogLevel.WARN, message, throwable)
    fun i(message: String) = write(LogLevel.INFO, message, null)
    fun d(message: String) = write(LogLevel.DEBUG, message, null)
    fun t(message: String) = write(LogLevel.TRACE, message, null)

    /** Returns a copy of the in-memory buffer, oldest first, for the diagnostics export. */
    fun snapshot(): List<String> = synchronized(lines) { lines.toList() }

    fun clear() {
        synchronized(lines) { lines.clear() }
        runCatching { logFile?.writeText("") }
    }

    /**
     * Redacts anything that looks like a credential.
     *
     * Secrets are 32 or 64 lowercase hex characters; tokens are 32. Matching on
     * shape rather than on field names means a secret cannot leak through a log
     * line that forgot to declare itself sensitive.
     */
    fun redact(message: String): String {
        var result = message
        result = HEX_32.replace(result) { "${it.value.take(4)}…[redacted]" }
        result = HEX_64.replace(result) { "${it.value.take(4)}…[redacted]" }
        result = AUTH_PREFIX.replace(result) { "${it.groupValues[1]}…[redacted]" }
        return result
    }

    private fun write(minLevel: LogLevel, message: String, throwable: Throwable?) {
        if (minLevel.order > level.order) return
        val safe = redact(message)
        val stamp = timestampFormat.format(Date())
        val line = "$stamp ${minLevel.label} $safe"
        android.util.Log.println(android.util.Log.INFO, TAG, line)

        synchronized(lines) {
            if (lines.size >= MAX_MEMORY_LINES) lines.removeFirst()
            lines.addLast(line)
        }
        listener?.invoke(line)

        runCatching {
            val file = logFile ?: return
            if (file.length() > MAX_FILE_BYTES) file.writeText("")
            file.appendText(line + "\n")
        }

        throwable?.let { android.util.Log.println(android.util.Log.INFO, TAG, android.util.Log.getStackTraceString(it)) }
    }

    private val HEX_32 = Regex("\\b[0-9a-f]{32}\\b")
    private val HEX_64 = Regex("\\b[0-9a-f]{64}\\b")
    private val AUTH_PREFIX = Regex("\\b(android-[0-9a-f]{8}:[a-z]*:?)([0-9a-f]{32,64})\\b")
}

/** Battery and thermal reporting for the PRD §26 warning and the §48 controller. */
class DeviceHealth(private val context: Context) {

    private val powerManager = ContextCompat.getSystemService(context, PowerManager::class.java)
    private val batteryManager = ContextCompat.getSystemService(context, android.os.BatteryManager::class.java)

    data class Snapshot(
        val batteryPercent: Int,
        val charging: Boolean,
        val thermal: String,
    )

    fun snapshot(): Snapshot {
        val percent = batteryManager?.getIntProperty(android.os.BatteryManager.BATTERY_PROPERTY_CAPACITY) ?: -1
        val charging = batteryManager?.isCharging == true
        return Snapshot(
            batteryPercent = percent.coerceIn(-1, 100),
            charging = charging,
            thermal = thermalState(),
        )
    }

    /** Maps `PowerManager.getCurrentThermalStatus` onto the wire vocabulary. */
    private fun thermalState(): String {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) return "none"
        return when (powerManager?.currentThermalStatus) {
            PowerManager.THERMAL_STATUS_NONE -> "none"
            PowerManager.THERMAL_STATUS_LIGHT -> "light"
            PowerManager.THERMAL_STATUS_MODERATE -> "moderate"
            PowerManager.THERMAL_STATUS_SEVERE -> "severe"
            PowerManager.THERMAL_STATUS_CRITICAL -> "critical"
            PowerManager.THERMAL_STATUS_EMERGENCY -> "emergency"
            PowerManager.THERMAL_STATUS_SHUTDOWN -> "shutdown"
            else -> "none"
        }
    }

    /** The PRD §26 advice shown when the phone is running hot. */
    fun thermalAdvice(thermal: String): String? = when (thermal) {
        "severe", "critical", "emergency", "shutdown" ->
            "Phone temperature is getting high.\n\nConsider:\n• lowering resolution\n• lowering FPS\n• connecting charger"
        "moderate" ->
            "Your phone is warming up. Lowering quality will keep the stream stable."
        else -> null
    }

    /** Battery warning shown while streaming without a charger. */
    fun batteryAdvice(percent: Int, charging: Boolean): String? = when {
        charging -> null
        percent in 0..10 -> "Phone battery is very low ($percent%). Connect a charger to keep streaming."
        percent in 11..20 -> "Phone battery is low ($percent%). Consider connecting a charger."
        else -> null
    }
}

/**
 * A bug report the user can actually send.
 *
 * The log alone is not enough to act on. "stream stopped" at 14:02 tells nobody
 * anything; the same line next to the device model, the Android version, which
 * encoder was chosen, whether the phone was on Wi-Fi or a cable, and the
 * battery and thermal state at the time is a report that can be reproduced.
 *
 * Everything here is gathered at the moment of the report rather than kept
 * live, because most of it is only interesting when something has already gone
 * wrong. It is also the reason the report is built from [Log.snapshot] and not
 * from the log file: the file can be truncated by the size cap and the buffer
 * cannot, so the buffer is the more complete of the two.
 */
object BugReport {

    private const val SEPARATOR = "----------------------------------------"

    /**
     * Builds the report body.
     *
     * `appVersion` is passed in rather than read from `BuildConfig` here so this
     * file stays free of the generated class and can be unit tested.
     */
    fun build(
        appVersion: String,
        deviceId: String,
        deviceName: String,
        device: ReportDevice,
        session: ReportSession,
    ): String {
        val builder = StringBuilder()

        builder.appendLine("PhoneCam bug report")
        builder.appendLine(SEPARATOR)
        builder.appendLine("App version   : $appVersion")
        builder.appendLine("Device id     : ${Log.redact(deviceId)}")
        builder.appendLine("Device name   : $deviceName")
        builder.appendLine()

        builder.appendLine("Device")
        builder.appendLine(SEPARATOR)
        builder.appendLine("Manufacturer  : ${device.manufacturer}")
        builder.appendLine("Model         : ${device.model}")
        builder.appendLine("Android       : ${device.androidRelease} (API ${device.sdkInt})")
        builder.appendLine("ABI           : ${device.abis}")
        builder.appendLine()

        builder.appendLine("Session at the time of the report")
        builder.appendLine(SEPARATOR)
        builder.appendLine("Phase         : ${session.phase}")
        // A transport is a fact about a link, not about the phone. The session
        // defaults it to `"lan"` and only `connectedTransport()` knows whether a
        // link is really up, so a `null` here means there is no link and the
        // report says so rather than printing the default as a LAN connection
        // that does not exist. Same honesty the Status card already applies.
        builder.appendLine("Transport     : ${session.transport ?: "Not connected"}")
        builder.appendLine("Camera        : ${session.camera}")
        builder.appendLine("Requested     : ${session.requestedResolution} @ ${session.requestedFps}")
        builder.appendLine("Actual        : ${session.actualResolution} @ ${session.actualFps}")
        builder.appendLine("Encoder       : ${session.encoder}")
        // The other reading the phone does not always have: `-1` is the session's
        // "not measured" sentinel, and printing "-1%" in a report reads as a
        // measurement of minus one percent. The guard lives here, in the builder,
        // so every caller is covered rather than each one remembering to check.
        builder.appendLine(
            if (session.batteryPercent < 0) {
                "Battery       : not measured"
            } else {
                "Battery       : ${session.batteryPercent}%${if (session.charging) " (charging)" else ""}"
            },
        )
        builder.appendLine("Thermal       : ${session.thermal}")
        builder.appendLine()

        builder.appendLine("Log")
        builder.appendLine(SEPARATOR)
        val lines = Log.snapshot()
        if (lines.isEmpty()) {
            builder.appendLine("(empty)")
        } else {
            // Every line is passed through the redactor again. It already ran on
            // the way in, but a report leaves the device and a credential in a
            // bug tracker is a real cost, so the last step before it leaves is
            // the one that must not be skipped.
            lines.forEach { builder.appendLine(Log.redact(it)) }
        }

        return builder.toString()
    }
}

/**
 * The hardware facts a report needs.
 *
 * Passed in rather than read from `Build` inside [BugReport] so the report can be
 * built and asserted on a plain JVM. `Build.SUPPORTED_ABIS` is null in a unit
 * test, and a report builder that cannot be tested is a report builder whose
 * redaction is never checked.
 */
data class ReportDevice(
    val manufacturer: String,
    val model: String,
    val androidRelease: String,
    val sdkInt: Int,
    val abis: String,
) {
    companion object {
        /** Reads the values from the platform. Only call this on a device. */
        fun current(): ReportDevice = ReportDevice(
            manufacturer = Build.MANUFACTURER,
            model = Build.MODEL,
            androidRelease = Build.VERSION.RELEASE,
            sdkInt = Build.VERSION.SDK_INT,
            abis = Build.SUPPORTED_ABIS.joinToString(", "),
        )
    }
}

/** The session facts a report needs, kept separate so the report has no dependencies. */
data class ReportSession(
    val phase: String,
    /**
     * The transport of a link that is up, or `null` when there is none.
     *
     * Callers pass [com.phonecam.camera.session.SessionState.connectedTransport],
     * not the raw [com.phonecam.camera.session.SessionState.transport], because
     * the raw field is set the moment a dial starts and defaults to `"lan"` on a
     * phone that has never connected.
     */
    val transport: String?,
    val camera: String,
    val requestedResolution: String,
    val requestedFps: String,
    val actualResolution: String,
    val actualFps: String,
    val encoder: String,
    val batteryPercent: Int,
    val charging: Boolean,
    val thermal: String,
)
