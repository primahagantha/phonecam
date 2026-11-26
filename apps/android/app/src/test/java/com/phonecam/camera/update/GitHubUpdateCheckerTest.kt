package com.phonecam.camera.update

import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.BufferedReader
import java.io.InputStreamReader
import java.net.InetAddress
import java.net.ServerSocket
import java.util.concurrent.atomic.AtomicReference

private class StubHttpServer(
    private val routes: Map<String, Pair<Int, String>>,
) : AutoCloseable {
    val serverSocket = ServerSocket(0, 16, InetAddress.getByName("127.0.0.1"))
    val lastAcceptHeader = AtomicReference<String?>(null)
    val lastUserAgentHeader = AtomicReference<String?>(null)
    @Volatile
    private var running = true

    private val thread = Thread({
        while (running) {
            val client = try {
                serverSocket.accept()
            } catch (_: Exception) {
                break
            }
            try {
                client.use { sock ->
                    val reader = BufferedReader(InputStreamReader(sock.getInputStream(), Charsets.UTF_8))
                    val requestLine = reader.readLine() ?: return@use
                    val path = requestLine.split(" ").getOrNull(1) ?: "/"
                    while (true) {
                        val line = reader.readLine() ?: break
                        if (line.isEmpty()) break
                        val idx = line.indexOf(':')
                        if (idx > 0) {
                            val key = line.substring(0, idx).trim()
                            val value = line.substring(idx + 1).trim()
                            if (key.equals("Accept", ignoreCase = true)) {
                                lastAcceptHeader.set(value)
                            } else if (key.equals("User-Agent", ignoreCase = true)) {
                                lastUserAgentHeader.set(value)
                            }
                        }
                    }
                    val (status, body) = routes[path] ?: (404 to "")
                    val bodyBytes = body.toByteArray(Charsets.UTF_8)
                    val responseHead = buildString {
                        append("HTTP/1.1 ").append(status).append(" OK\r\n")
                        append("Content-Type: application/json\r\n")
                        append("Content-Length: ").append(bodyBytes.size).append("\r\n")
                        append("Connection: close\r\n\r\n")
                    }
                    val out = sock.getOutputStream()
                    out.write(responseHead.toByteArray(Charsets.US_ASCII))
                    if (bodyBytes.isNotEmpty()) {
                        out.write(bodyBytes)
                    }
                    out.flush()
                }
            } catch (_: Exception) {
                // Ignore client reset during teardown
            }
        }
    }, "stub-github-http").apply {
        isDaemon = true
        start()
    }

    val baseUrl: String
        get() = "http://127.0.0.1:${serverSocket.localPort}"

    override fun close() {
        running = false
        runCatching { serverSocket.close() }
        runCatching { thread.join(1000L) }
    }
}

class GitHubUpdateCheckerTest {

    // --------------------------------------------------------- SemVer comparisons

    @Test
    fun `isNewerVersion handles standard major minor patch increments`() {
        assertTrue(GitHubUpdateChecker.isNewerVersion("1.4.0", "1.4.1"))
        assertTrue(GitHubUpdateChecker.isNewerVersion("1.4.0", "1.5.0"))
        assertTrue(GitHubUpdateChecker.isNewerVersion("1.4.9", "1.10.0"))
        assertTrue(GitHubUpdateChecker.isNewerVersion("1.9.9", "2.0.0"))

        assertFalse(GitHubUpdateChecker.isNewerVersion("1.4.0", "1.4.0"))
        assertFalse(GitHubUpdateChecker.isNewerVersion("1.4.1", "1.4.0"))
        assertFalse(GitHubUpdateChecker.isNewerVersion("2.0.0", "1.9.9"))
    }

    @Test
    fun `isNewerVersion strips leading v prefixes and whitespace`() {
        assertTrue(GitHubUpdateChecker.isNewerVersion("v1.4.0", "v1.4.1"))
        assertTrue(GitHubUpdateChecker.isNewerVersion("1.4.0", "v1.4.1"))
        assertTrue(GitHubUpdateChecker.isNewerVersion("v1.4.0", "1.4.1"))
        assertTrue(GitHubUpdateChecker.isNewerVersion("  V1.4.0 ", " v1.5.0  "))

        assertFalse(GitHubUpdateChecker.isNewerVersion("v1.4.0", "1.4.0"))
        assertFalse(GitHubUpdateChecker.isNewerVersion("V1.4.1", "v1.4.0"))
    }

    @Test
    fun `isNewerVersion ranks prereleases below final release per SemVer 2_0_0`() {
        // Core requirement: 1.4.0-nightly.1 < 1.4.0 < 1.4.1
        assertTrue(GitHubUpdateChecker.isNewerVersion("1.4.0-nightly.1", "1.4.0"))
        assertTrue(GitHubUpdateChecker.isNewerVersion("1.4.0", "1.4.1"))
        assertTrue(GitHubUpdateChecker.isNewerVersion("1.4.0-nightly.1", "1.4.1"))
        assertFalse(GitHubUpdateChecker.isNewerVersion("1.4.0", "1.4.0-nightly.1"))
        assertFalse(GitHubUpdateChecker.isNewerVersion("1.4.1", "1.4.0"))

        // Debug suffix behaves as a pre-release of the same version
        assertTrue(GitHubUpdateChecker.isNewerVersion("1.4.0-debug", "v1.4.0"))
        assertFalse(GitHubUpdateChecker.isNewerVersion("1.4.0", "1.4.0-debug"))
    }

    @Test
    fun `isNewerVersion compares prerelease identifiers numerically and lexically`() {
        // Numeric comparison: nightly.2 < nightly.10
        assertTrue(GitHubUpdateChecker.isNewerVersion("1.4.0-nightly.2", "1.4.0-nightly.10"))
        assertFalse(GitHubUpdateChecker.isNewerVersion("1.4.0-nightly.10", "1.4.0-nightly.2"))

        // Lexical comparison: alpha < beta < nightly < rc
        assertTrue(GitHubUpdateChecker.isNewerVersion("1.4.0-alpha.1", "1.4.0-beta.1"))
        assertTrue(GitHubUpdateChecker.isNewerVersion("1.4.0-beta.2", "1.4.0-nightly.1"))
        assertTrue(GitHubUpdateChecker.isNewerVersion("1.4.0-nightly.5", "1.4.0-rc.1"))
        assertTrue(GitHubUpdateChecker.isNewerVersion("1.4.0-rc.1", "1.4.0"))

        // More identifiers has higher precedence when prefix matches
        assertTrue(GitHubUpdateChecker.isNewerVersion("1.4.0-alpha", "1.4.0-alpha.1"))
        assertFalse(GitHubUpdateChecker.isNewerVersion("1.4.0-alpha.1", "1.4.0-alpha"))

        // Numeric identifier has lower precedence than non-numeric identifier
        assertTrue(GitHubUpdateChecker.isNewerVersion("1.4.0-1", "1.4.0-alpha"))
        assertFalse(GitHubUpdateChecker.isNewerVersion("1.4.0-alpha", "1.4.0-1"))

        // Newer minor pre-release is newer than older stable
        assertTrue(GitHubUpdateChecker.isNewerVersion("1.4.0", "1.5.0-rc.1"))
    }

    @Test
    fun `isNewerVersion ignores build metadata and rejects malformed strings safely`() {
        assertFalse(GitHubUpdateChecker.isNewerVersion("1.4.0+build.1", "1.4.0+build.2"))
        assertTrue(GitHubUpdateChecker.isNewerVersion("1.4.0+sha.abc", "1.4.1+sha.def"))

        assertFalse(GitHubUpdateChecker.isNewerVersion("", "1.4.0"))
        assertFalse(GitHubUpdateChecker.isNewerVersion("1.4.0", ""))
        assertFalse(GitHubUpdateChecker.isNewerVersion("not-a-version", "1.4.0"))
        assertFalse(GitHubUpdateChecker.isNewerVersion("1.4.0", "v-invalid"))
        assertFalse(GitHubUpdateChecker.isNewerVersion("1.4.0", "1.4.0-"))
        assertFalse(GitHubUpdateChecker.isNewerVersion("1.4.0", "1.4.0-alpha..1"))
    }

    @Test
    fun `formatDisplayVersion and formatPublishedDate normalize values cleanly`() {
        assertEquals("v1.4.0", GitHubUpdateChecker.formatDisplayVersion("1.4.0"))
        assertEquals("v1.4.0", GitHubUpdateChecker.formatDisplayVersion("v1.4.0"))
        assertEquals("v1.4.0", GitHubUpdateChecker.formatDisplayVersion("V1.4.0"))
        assertEquals("2026-11-26", GitHubUpdateChecker.formatPublishedDate("2026-11-26T16:30:00Z"))
        assertEquals("", GitHubUpdateChecker.formatPublishedDate(""))
    }

    // --------------------------------------------------------- JSON parsing

    @Test
    fun `parseReleaseJson extracts fields and ignores unknown GitHub properties`() {
        val json = """
            {
              "id": 123456,
              "tag_name": "v1.5.0",
              "name": "PhoneCam Release v1.5.0",
              "body": "## What's Changed\n- Added update checker\n- Polished UI",
              "html_url": "https://github.com/primahagantha/phonecam/releases/tag/v1.5.0",
              "published_at": "2026-11-26T12:00:00Z",
              "draft": false,
              "prerelease": false,
              "assets": []
            }
        """.trimIndent()

        val release = GitHubUpdateChecker.parseReleaseJson(json)
        assertNotNull(release)
        assertEquals("v1.5.0", release!!.tagName)
        assertEquals("PhoneCam Release v1.5.0", release.name)
        assertTrue(release.body.contains("Added update checker"))
        assertEquals("https://github.com/primahagantha/phonecam/releases/tag/v1.5.0", release.htmlUrl)
        assertEquals("2026-11-26T12:00:00Z", release.publishedAt)
    }

    @Test
    fun `parseReleaseJson handles missing optional fields and rejects unsafe URLs`() {
        val minimal = """
            {
              "tag_name": "v1.5.0",
              "html_url": "https://github.com/primahagantha/phonecam/releases/tag/v1.5.0"
            }
        """.trimIndent()
        val parsed = GitHubUpdateChecker.parseReleaseJson(minimal)
        assertNotNull(parsed)
        assertEquals("v1.5.0", parsed!!.name)
        assertEquals("", parsed.body)
        assertEquals("", parsed.publishedAt)

        val unsafeUrl = """
            {
              "tag_name": "v1.5.0",
              "html_url": "javascript:alert(1)"
            }
        """.trimIndent()
        assertNull(GitHubUpdateChecker.parseReleaseJson(unsafeUrl))

        val invalidTag = """
            {
              "tag_name": "not-semver",
              "html_url": "https://github.com/primahagantha/phonecam/releases/tag/bad"
            }
        """.trimIndent()
        assertNull(GitHubUpdateChecker.parseReleaseJson(invalidTag))
        assertNull(GitHubUpdateChecker.parseReleaseJson("{invalid json"))
    }

    // --------------------------------------------------------- HTTP client behavior

    @Test
    fun `checkForUpdates sends expected GitHub headers and detects newer release`() = runTest {
        val body = """
            {
              "tag_name": "v1.5.0",
              "name": "PhoneCam v1.5.0",
              "body": "- Faster camera switching\n- Update checker",
              "html_url": "https://github.com/primahagantha/phonecam/releases/tag/v1.5.0",
              "published_at": "2026-11-26T10:00:00Z"
            }
        """.trimIndent()
        StubHttpServer(mapOf("/releases/latest" to (200 to body))).use { server ->
            val result = GitHubUpdateChecker.checkForUpdates(
                currentVersion = "1.4.0",
                endpointUrl = "${server.baseUrl}/releases/latest",
            )

            assertEquals("application/vnd.github+json", server.lastAcceptHeader.get())
            assertEquals("PhoneCam-Android/1.4.0", server.lastUserAgentHeader.get())
            assertTrue("expected UpdateAvailable, got $result", result is UpdateCheckState.UpdateAvailable)
            val available = result as UpdateCheckState.UpdateAvailable
            assertEquals("v1.5.0", available.release.tagName)
            assertTrue(available.release.body.contains("Faster camera switching"))
        }
    }

    @Test
    fun `checkForUpdates reports UpToDate when current matches or exceeds latest`() = runTest {
        val body = """
            {
              "tag_name": "v1.4.0",
              "name": "PhoneCam v1.4.0",
              "body": "Initial release",
              "html_url": "https://github.com/primahagantha/phonecam/releases/tag/v1.4.0",
              "published_at": "2026-11-20T10:00:00Z"
            }
        """.trimIndent()
        StubHttpServer(mapOf("/releases/latest" to (200 to body))).use { server ->
            val result = GitHubUpdateChecker.checkForUpdates(
                currentVersion = "1.4.0",
                endpointUrl = "${server.baseUrl}/releases/latest",
            )
            assertTrue("expected UpToDate, got $result", result is UpdateCheckState.UpToDate)
            assertEquals("v1.4.0", (result as UpdateCheckState.UpToDate).latestTag)
        }
    }

    @Test
    fun `checkForUpdates maps HTTP 404 403 500 and invalid JSON to typed errors`() = runTest {
        StubHttpServer(
            mapOf(
                "/404" to (404 to ""),
                "/403" to (403 to ""),
                "/500" to (500 to ""),
                "/bad" to (200 to "not json"),
            ),
        ).use { server ->
            val base = server.baseUrl
            assertEquals(
                UpdateCheckState.Error(UpdateErrorReason.NoRelease),
                GitHubUpdateChecker.checkForUpdates("1.4.0", "$base/404"),
            )
            assertEquals(
                UpdateCheckState.Error(UpdateErrorReason.RateLimited),
                GitHubUpdateChecker.checkForUpdates("1.4.0", "$base/403"),
            )
            assertEquals(
                UpdateCheckState.Error(UpdateErrorReason.ServerError, detail = "500"),
                GitHubUpdateChecker.checkForUpdates("1.4.0", "$base/500"),
            )
            assertEquals(
                UpdateCheckState.Error(UpdateErrorReason.InvalidResponse),
                GitHubUpdateChecker.checkForUpdates("1.4.0", "$base/bad"),
            )
        }
    }
}
