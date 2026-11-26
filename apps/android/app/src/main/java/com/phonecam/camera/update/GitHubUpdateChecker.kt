package com.phonecam.camera.update

import com.phonecam.camera.protocol.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.Json
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL

/**
 * Metadata for the latest public GitHub Release.
 */
data class ReleaseInfo(
    val tagName: String,
    val name: String,
    val body: String,
    val htmlUrl: String,
    val publishedAt: String,
)

/**
 * Reason an update check did not complete with a release comparison.
 *
 * Mapped to localized strings by the UI so errors read naturally in both
 * English and Indonesian without exposing raw exception class names.
 */
enum class UpdateErrorReason {
    Network,
    RateLimited,
    NoRelease,
    ServerError,
    InvalidResponse,
}

/**
 * UI-facing state of a user-initiated update check.
 */
sealed interface UpdateCheckState {
    data object Idle : UpdateCheckState
    data object Checking : UpdateCheckState
    data class UpToDate(val currentVersion: String, val latestTag: String) : UpdateCheckState
    data class UpdateAvailable(val currentVersion: String, val release: ReleaseInfo) : UpdateCheckState
    data class Error(val reason: UpdateErrorReason, val detail: String? = null) : UpdateCheckState
}

@Serializable
internal data class GitHubReleasePayload(
    @SerialName("tag_name") val tagName: String = "",
    @SerialName("name") val name: String? = null,
    @SerialName("body") val body: String? = null,
    @SerialName("html_url") val htmlUrl: String = "",
    @SerialName("published_at") val publishedAt: String? = null,
)

/**
 * Parsed Semantic Version 2.0.0 representation (`MAJOR.MINOR.PATCH[-PRERELEASE]`).
 */
internal data class SemVer(
    val major: Int,
    val minor: Int,
    val patch: Int,
    val preRelease: List<String> = emptyList(),
) : Comparable<SemVer> {

    override fun compareTo(other: SemVer): Int {
        if (major != other.major) return major.compareTo(other.major)
        if (minor != other.minor) return minor.compareTo(other.minor)
        if (patch != other.patch) return patch.compareTo(other.patch)

        // SemVer 2.0.0 §11.3: a normal version has higher precedence than a pre-release.
        if (preRelease.isEmpty() && other.preRelease.isEmpty()) return 0
        if (preRelease.isEmpty()) return 1
        if (other.preRelease.isEmpty()) return -1

        val count = minOf(preRelease.size, other.preRelease.size)
        for (i in 0 until count) {
            val a = preRelease[i]
            val b = other.preRelease[i]
            if (a == b) continue

            val aNum = a.toBigIntegerOrNull()
            val bNum = b.toBigIntegerOrNull()
            return when {
                aNum != null && bNum != null -> aNum.compareTo(bNum)
                // SemVer 2.0.0 §11.4.3: numeric identifiers always have lower precedence
                // than non-numeric identifiers.
                aNum != null && bNum == null -> -1
                aNum == null && bNum != null -> 1
                else -> a.compareTo(b)
            }
        }

        // SemVer 2.0.0 §11.4.4: a larger set of pre-release fields has higher precedence.
        return preRelease.size.compareTo(other.preRelease.size)
    }

    companion object {
        private val CORE_PART_REGEX = Regex("^[0-9]+$")

        fun parse(raw: String): SemVer? {
            val trimmed = raw.trim()
            if (trimmed.isEmpty()) return null

            val withoutPrefix = if (trimmed.startsWith('v') || trimmed.startsWith('V')) {
                trimmed.substring(1)
            } else {
                trimmed
            }
            if (withoutPrefix.isEmpty()) return null

            // SemVer 2.0.0 §10: build metadata does not affect precedence.
            val withoutBuild = withoutPrefix.substringBefore('+')
            val dashIndex = withoutBuild.indexOf('-')
            val corePart = if (dashIndex >= 0) withoutBuild.substring(0, dashIndex) else withoutBuild
            val prePart = if (dashIndex >= 0) withoutBuild.substring(dashIndex + 1) else ""

            val coreSegments = corePart.split('.')
            if (coreSegments.isEmpty() || coreSegments.size > 3) return null
            if (coreSegments.any { !CORE_PART_REGEX.matches(it) }) return null

            val major = coreSegments[0].toIntOrNull() ?: return null
            val minor = coreSegments.getOrNull(1)?.toIntOrNull() ?: 0
            val patch = coreSegments.getOrNull(2)?.toIntOrNull() ?: 0

            val preIdentifiers = if (dashIndex >= 0) {
                if (prePart.isEmpty()) return null
                val parts = prePart.split('.')
                if (parts.any { it.isEmpty() }) return null
                parts
            } else {
                emptyList()
            }

            return SemVer(
                major = major,
                minor = minor,
                patch = patch,
                preRelease = preIdentifiers,
            )
        }
    }
}

/**
 * User-initiated GitHub Releases update checker.
 *
 * Strictly opt-in: this checker is ONLY invoked when the user explicitly taps
 * "Check for Updates". It never polls, schedules background work, or transmits
 * device identifiers or telemetry.
 */
object GitHubUpdateChecker {
    const val RELEASES_API_URL = "https://api.github.com/repos/primahagantha/phonecam/releases/latest"
    const val TIMEOUT_MS = 8_000
    private const val MAX_RESPONSE_BYTES = 512 * 1024

    private val json = Json {
        ignoreUnknownKeys = true
        coerceInputValues = true
    }

    /**
     * Returns `true` if [latest] is strictly newer than [current] according to SemVer 2.0.0.
     *
     * Handles optional `v`/`V` prefixes, `X.Y.Z` cores, build metadata (`+...`),
     * and pre-release identifiers (`1.4.0-nightly.1 < 1.4.0 < 1.4.1`).
     */
    fun isNewerVersion(current: String, latest: String): Boolean {
        val currentVer = SemVer.parse(current) ?: return false
        val latestVer = SemVer.parse(latest) ?: return false
        return latestVer > currentVer
    }

    /**
     * Formats a version string consistently with a single leading `v` (e.g. `v1.4.0`).
     */
    fun formatDisplayVersion(version: String): String {
        val trimmed = version.trim()
        return if (trimmed.startsWith('v') || trimmed.startsWith('V')) {
            "v${trimmed.substring(1)}"
        } else {
            "v$trimmed"
        }
    }

    /**
     * Extracts a clean `YYYY-MM-DD` date from a GitHub ISO-8601 `published_at` timestamp,
     * returning an empty string when unavailable.
     */
    fun formatPublishedDate(publishedAt: String): String {
        val trimmed = publishedAt.trim()
        if (trimmed.length >= 10 && trimmed[4] == '-' && trimmed[7] == '-') {
            return trimmed.substring(0, 10)
        }
        return trimmed
    }

    internal fun parseReleaseJson(rawJson: String): ReleaseInfo? {
        val payload = try {
            json.decodeFromString(GitHubReleasePayload.serializer(), rawJson)
        } catch (_: SerializationException) {
            return null
        } catch (_: IllegalArgumentException) {
            return null
        }

        val tag = payload.tagName.trim()
        val url = payload.htmlUrl.trim()
        if (tag.isEmpty() || SemVer.parse(tag) == null) return null
        if (!isSafeReleaseUrl(url)) return null

        val releaseName = payload.name?.trim().orEmpty().ifEmpty { tag }
        val releaseBody = payload.body?.trim().orEmpty()
        val published = payload.publishedAt?.trim().orEmpty()

        return ReleaseInfo(
            tagName = tag,
            name = releaseName,
            body = releaseBody,
            htmlUrl = url,
            publishedAt = published,
        )
    }

    internal fun isSafeReleaseUrl(url: String): Boolean =
        url.startsWith("https://", ignoreCase = true)

    /**
     * Queries GitHub Releases for the latest version and compares it with [currentVersion].
     *
     * Runs on [Dispatchers.IO] using standard [HttpURLConnection] with 8-second
     * connect and read timeouts.
     */
    suspend fun checkForUpdates(
        currentVersion: String,
        endpointUrl: String = RELEASES_API_URL,
    ): UpdateCheckState = withContext(Dispatchers.IO) {
        var connection: HttpURLConnection? = null
        try {
            val url = URL(endpointUrl)
            connection = (url.openConnection() as HttpURLConnection).apply {
                requestMethod = "GET"
                connectTimeout = TIMEOUT_MS
                readTimeout = TIMEOUT_MS
                instanceFollowRedirects = true
                useCaches = false
                setRequestProperty("Accept", "application/vnd.github+json")
                setRequestProperty("User-Agent", "PhoneCam-Android/$currentVersion")
            }

            val code = connection.responseCode
            when (code) {
                HttpURLConnection.HTTP_OK -> {
                    val body = connection.inputStream.use { stream ->
                        readBoundedUtf8(stream, MAX_RESPONSE_BYTES)
                    }
                    val release = parseReleaseJson(body)
                    if (release == null) {
                        Log.w("update check: invalid GitHub release payload")
                        UpdateCheckState.Error(UpdateErrorReason.InvalidResponse)
                    } else if (isNewerVersion(currentVersion, release.tagName)) {
                        Log.i("update check: newer release available (${release.tagName})")
                        UpdateCheckState.UpdateAvailable(
                            currentVersion = currentVersion,
                            release = release,
                        )
                    } else {
                        Log.i("update check: up to date (latest=${release.tagName})")
                        UpdateCheckState.UpToDate(
                            currentVersion = currentVersion,
                            latestTag = release.tagName,
                        )
                    }
                }

                HttpURLConnection.HTTP_NOT_FOUND -> {
                    Log.i("update check: no published GitHub release found (404)")
                    UpdateCheckState.Error(UpdateErrorReason.NoRelease)
                }

                HttpURLConnection.HTTP_FORBIDDEN, 429 -> {
                    Log.w("update check: GitHub API rate limited ($code)")
                    UpdateCheckState.Error(UpdateErrorReason.RateLimited)
                }

                else -> {
                    Log.w("update check: unexpected HTTP status $code")
                    UpdateCheckState.Error(UpdateErrorReason.ServerError, detail = code.toString())
                }
            }
        } catch (e: IOException) {
            Log.w("update check: network error (${e.javaClass.simpleName})")
            UpdateCheckState.Error(UpdateErrorReason.Network)
        } catch (e: SecurityException) {
            Log.w("update check: network permission error (${e.javaClass.simpleName})")
            UpdateCheckState.Error(UpdateErrorReason.Network)
        } finally {
            connection?.disconnect()
        }
    }

    private fun readBoundedUtf8(stream: java.io.InputStream, maxBytes: Int): String {
        val buffer = ByteArray(4096)
        val out = java.io.ByteArrayOutputStream()
        var total = 0
        while (true) {
            val read = stream.read(buffer)
            if (read <= 0) break
            val toWrite = minOf(read, maxBytes - total)
            if (toWrite > 0) {
                out.write(buffer, 0, toWrite)
                total += toWrite
            }
            if (total >= maxBytes) break
        }
        return out.toString(Charsets.UTF_8.name())
    }
}
