package com.phonecam.camera.transport

import com.phonecam.camera.protocol.AuthMode
import com.phonecam.camera.protocol.ChannelRole
import com.phonecam.camera.protocol.HandshakeStatus
import com.phonecam.camera.protocol.ServerResponse
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import java.net.Socket

/**
 * The rule that turns the desktop's handshake status into the outcome the
 * session acts on.
 *
 * The phone's recovery from "the PC no longer knows me" hung on this mapping:
 * the core answers an unknown device id on a device-secret dial with `denied`,
 * but `denied` was mapped to [DialResult.Denied], whose copy tells the user to
 * approve a request that was never made. A human denial never arrives as a
 * handshake `denied` at all — it comes as a `pair_denied` control message on the
 * approval socket — so a device-secret `denied` is free to mean "pair again".
 * The rule is kept pure so this cannot regress unnoticed: a live dial to a PC
 * that has forgotten the phone is not available to a unit test, but the decision
 * that was wrong is.
 */
class DialResultMappingTest {

    private fun connection(): PeerConnection = PeerConnection(
        socket = Socket(),
        input = DataInputStream(ByteArrayInputStream(ByteArray(0))),
        output = DataOutputStream(ByteArrayOutputStream()),
        role = ChannelRole.CONTROL,
        response = ServerResponse(1, HandshakeStatus.OK, ChannelRole.CONTROL.wire, ""),
    )

    private fun map(status: HandshakeStatus, mode: AuthMode): DialResult =
        dialResultFor(status, mode) { connection() }

    @Test
    fun `a device-secret denial means the PC does not know this phone`() {
        // The core's only device-secret denials are "unknown device <id>", a bad
        // HMAC and a replayed nonce — each is "the stored secret is no longer
        // usable", so the phone must pair again rather than wait for an approval.
        assertEquals(
            DialResult.CredentialsRejected,
            map(HandshakeStatus.DENIED, AuthMode.DEVICE_SECRET),
        )
    }

    @Test
    fun `a device-secret dial reported as expired also means pair again`() {
        assertEquals(
            DialResult.CredentialsRejected,
            map(HandshakeStatus.TOKEN_EXPIRED, AuthMode.DEVICE_SECRET),
        )
    }

    @Test
    fun `a pairing-token denial stays a refusal`() {
        // A token is a freshly scanned QR code, not a stale secret: a refusal
        // here is genuine and must not be turned into a silent re-pair.
        assertEquals(DialResult.Denied, map(HandshakeStatus.DENIED, AuthMode.PAIRING_TOKEN))
    }

    @Test
    fun `a pairing-token dial reported as expired means pair again`() {
        assertEquals(
            DialResult.CredentialsRejected,
            map(HandshakeStatus.TOKEN_EXPIRED, AuthMode.PAIRING_TOKEN),
        )
    }

    @Test
    fun `a pair-request denial is not a credential rejection`() {
        // A pair request carries no stored credential to reject, and the core
        // answers an unknown device on this mode with the fixed pair-request key
        // rather than a denial, so `denied` here is a bad signature. Treating it
        // as CredentialsRejected would re-enter the recovery loop pointlessly.
        assertEquals(DialResult.Denied, map(HandshakeStatus.DENIED, AuthMode.PAIR_REQUEST))
    }

    @Test
    fun `an accepted handshake is Ok`() {
        assertTrue(map(HandshakeStatus.OK, AuthMode.DEVICE_SECRET) is DialResult.Ok)
    }

    @Test
    fun `a pending approval keeps the channel open`() {
        assertTrue(
            map(HandshakeStatus.APPROVAL_PENDING, AuthMode.PAIR_REQUEST)
                is DialResult.ApprovalPending,
        )
    }

    @Test
    fun `a busy desktop is busy, not a credential problem`() {
        assertEquals(DialResult.Busy, map(HandshakeStatus.BUSY, AuthMode.DEVICE_SECRET))
    }
}
