package com.phonecam.camera.protocol

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import com.phonecam.camera.BuildConfig
import java.io.File
import java.net.URLEncoder
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Persisted crash report metadata and utilities for reporting to GitHub.
 */
data class CrashReport(
    val timestamp: String,
    val exceptionClass: String,
    val message: String,
    val threadName: String,
    val stackTrace: String,
    val manufacturer: String,
    val model: String,
    val device: String,
    val androidRelease: String,
    val sdkInt: Int,
    val appVersion: String,
    val logs: List<String>,
)

object CrashReporter {
    private const val CRASH_FILE_NAME = "crash_report.txt"
    private const val GITHUB_REPO_OWNER = "primahagantha"
    private const val GITHUB_REPO_NAME = "phonecam"
    private const val TAG = "CrashReporter"

    private val dateFormat = SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS", Locale.US)

    /**
     * Captures and persists an unhandled exception before process termination.
     */
    fun recordCrash(context: Context, thread: Thread, error: Throwable) {
        runCatching {
            val file = File(context.filesDir, CRASH_FILE_NAME)
            val stamp = dateFormat.format(Date())
            val stack = android.util.Log.getStackTraceString(error)
            val logs = Log.snapshot().takeLast(60)

            val sb = StringBuilder()
            sb.appendLine("=== PHONECAM CRASH REPORT ===")
            sb.appendLine("Timestamp      : $stamp")
            sb.appendLine("App Version    : ${BuildConfig.VERSION_NAME} (${BuildConfig.VERSION_CODE})")
            sb.appendLine("Device Model   : ${Build.MANUFACTURER} ${Build.MODEL} (${Build.DEVICE})")
            sb.appendLine("Android OS     : ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT})")
            sb.appendLine("Thread         : ${thread.name} (id ${thread.id})")
            sb.appendLine("Exception      : ${error.javaClass.name}")
            sb.appendLine("Message        : ${error.message ?: "No message"}")
            sb.appendLine()
            sb.appendLine("=== STACK TRACE ===")
            sb.appendLine(stack)
            sb.appendLine()
            sb.appendLine("=== RECENT DIAGNOSTICS LOGS ===")
            if (logs.isEmpty()) {
                sb.appendLine("(no log lines recorded)")
            } else {
                for (line in logs) {
                    sb.appendLine(Log.redact(line))
                }
            }
            sb.appendLine("=== END OF REPORT ===")

            file.writeText(sb.toString())
        }
    }

    /**
     * Checks if a crash report from a previous run exists.
     */
    fun hasPendingCrashReport(context: Context): Boolean {
        val file = File(context.filesDir, CRASH_FILE_NAME)
        return file.exists() && file.length() > 0
    }

    /**
     * Reads the pending crash report text, if any.
     */
    fun readPendingCrashReport(context: Context): String? {
        val file = File(context.filesDir, CRASH_FILE_NAME)
        return if (file.exists() && file.length() > 0) {
            runCatching { file.readText() }.getOrNull()
        } else {
            null
        }
    }

    /**
     * Removes the pending crash report once dismissed or sent.
     */
    fun clearPendingCrashReport(context: Context) {
        runCatching {
            val file = File(context.filesDir, CRASH_FILE_NAME)
            if (file.exists()) {
                file.delete()
            }
        }
    }

    /**
     * Builds a pre-filled GitHub Issue URL for reporting crashes or general issues.
     */
    fun buildGitHubIssueIntent(
        context: Context,
        title: String,
        bodyMarkdown: String,
    ): Intent {
        val encodedTitle = runCatching { URLEncoder.encode(title, "UTF-8") }.getOrDefault(title)
        val encodedBody = runCatching {
            // Cap body query parameter to safe URI length (around 3500 chars) for Android browser intents
            val truncated = if (bodyMarkdown.length > 3500) {
                bodyMarkdown.take(3500) + "\n\n...(truncated, full crash report is copied to clipboard)"
            } else {
                bodyMarkdown
            }
            URLEncoder.encode(truncated, "UTF-8")
        }.getOrDefault(bodyMarkdown)

        val uriString = "https://github.com/$GITHUB_REPO_OWNER/$GITHUB_REPO_NAME/issues/new?title=$encodedTitle&body=$encodedBody"
        return Intent(Intent.ACTION_VIEW, Uri.parse(uriString)).apply {
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
    }
}
