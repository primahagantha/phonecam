package com.phonecam.camera.protocol

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The bug report is the artefact a user sends when something has gone wrong, so
 * two properties matter more than its exact wording: it must not carry a
 * credential off the device, and it must contain enough to act on.
 *
 * Both are asserted here rather than left to review, because the first is a
 * security property that a careless `appendLine` would silently undo, and the
 * second is the entire reason the report exists.
 */
class BugReportTest {

    private fun device() = ReportDevice(
        manufacturer = "Xiaomi",
        model = "2201117TG",
        androidRelease = "13",
        sdkInt = 33,
        abis = "arm64-v8a, armeabi-v7a",
    )

    private fun session() = ReportSession(
        phase = "streaming",
        transport = "lan",
        camera = "0",
        requestedResolution = "1920x1080",
        requestedFps = "30",
        actualResolution = "1280x720",
        actualFps = "30",
        encoder = "c2.qti.avc.encoder",
        batteryPercent = 74,
        charging = false,
        thermal = "light",
    )

    @Test
    fun `a device secret never reaches the report`() {
        val secret = "a".repeat(64)
        Log.clear()
        Log.i("paired with fingerprint $secret")
        val report = BugReport.build("0.1.0", "android-12345678", "Pixel", device(), session())
        assertFalse("a 64-character secret must not survive", report.contains(secret))
        assertTrue("the redaction must be visible", report.contains("[redacted]"))
    }

    @Test
    fun `a device id keeps its prefix and loses its secret`() {
        // The id is what makes two reports from the same phone comparable, so it
        // is kept; the secret after the colon is not.
        Log.clear()
        Log.i("connected as android-9c84583b:deadbeefdeadbeefdeadbeefdeadbeef")
        val report = BugReport.build("0.1.0", "android-9c84583b", "Pixel", device(), session())
        assertFalse(report.contains("deadbeefdeadbeefdeadbeefdeadbeef"))
        assertTrue(report.contains("android-9c84583b"))
    }

    @Test
    fun `the report names the device and the session it was taken from`() {
        // Without these the log lines have no context, and a report that cannot
        // be reproduced is not worth sending.
        Log.clear()
        Log.i("stream started")
        val report = BugReport.build("0.1.0", "android-12345678", "Pixel 7", device(), session())
        assertTrue(report.contains("0.1.0"))
        assertTrue(report.contains("Pixel 7"))
        assertTrue(report.contains("streaming"))
        assertTrue(report.contains("1280x720"))
        assertTrue(report.contains("c2.qti.avc.encoder"))
        assertTrue(report.contains("74%"))
        assertTrue(report.contains("light"))
    }

    @Test
    fun `an empty log still produces a readable report`() {
        // The most useful report is often the one from a fresh launch, where
        // nothing has been logged yet.
        Log.clear()
        val report = BugReport.build("0.1.0", "android-12345678", "Pixel 7", device(), session())
        assertTrue(report.contains("(empty)"))
        assertTrue(report.contains("Device"))
    }

    @Test
    fun `the requested and the actual stream shape are both reported`() {
        // The difference between the two is the whole story when the phone
        // silently substitutes a smaller resolution.
        Log.clear()
        val report = BugReport.build("0.1.0", "android-12345678", "Pixel 7", device(), session())
        assertTrue(report.contains("1920x1080"))
        assertTrue(report.contains("1280x720"))
    }

    @Test
    fun `a battery reading that was never taken is not printed as a number`() {
        // `batteryPercent` defaults to -1, the session's "not measured" sentinel.
        // Printed raw it reads as a battery at minus one percent, a measurement
        // nobody took. The guard lives in the builder, so it holds for the
        // fresh-launch report as well as every other caller.
        Log.clear()
        val report = BugReport.build(
            "0.1.0", "android-12345678", "Pixel 7", device(),
            session().copy(batteryPercent = -1, charging = false),
        )
        assertFalse("a report must not print a battery reading it does not have", report.contains("-1%"))
        assertTrue(report.contains("Battery       : not measured"))
    }

    @Test
    fun `a transport that is only a default is not printed as a link`() {
        // The session defaults `transport` to "lan", so a report built from the
        // raw field claimed a LAN link on a phone that had never connected. The
        // caller passes `connectedTransport()`, which is null outside a real link.
        Log.clear()
        val report = BugReport.build(
            "0.1.0", "android-12345678", "Pixel 7", device(),
            session().copy(transport = null),
        )
        assertTrue(report.contains("Transport     : Not connected"))
        assertFalse(report.contains("Transport     : lan"))
    }

    @Test
    fun `a real link and a real battery reading survive the guards`() {
        // The guards must not swallow a measurement that was actually taken.
        Log.clear()
        val report = BugReport.build("0.1.0", "android-12345678", "Pixel 7", device(), session())
        assertTrue(report.contains("Transport     : lan"))
        assertTrue(report.contains("Battery       : 74%"))
    }
}
