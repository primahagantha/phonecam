package com.phonecam.camera.transport

import android.util.Log
import com.phonecam.camera.data.IdentityStore
import com.phonecam.camera.protocol.Auth
import com.phonecam.camera.protocol.AuthMode
import com.phonecam.camera.protocol.ChannelRole
import com.phonecam.camera.protocol.Framing
import com.phonecam.camera.protocol.HandshakeStatus
import com.phonecam.camera.protocol.Proto
import com.phonecam.camera.protocol.ServerResponse
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.IOException
import java.net.InetSocketAddress
import java.net.Socket
import java.net.SocketTimeoutException

/**
 * The outcome of a dial attempt, expressed in terms the UI can show (PRD §38).
 *
 * Every variant maps to human copy; the UI never renders a raw exception, and an
 * expected condition is never presented as a modal error (§39).
 */
sealed class DialResult {
    /** Handshake accepted; [connection] is ready for framed traffic. */
    data class Ok(val connection: PeerConnection) : DialResult()

    /**
     * A `pairRequest` reached the desktop and a human prompt is now showing.
     * The connection stays open so the decision can arrive over it (§4.3).
     */
    data class ApprovalPending(val connection: PeerConnection) : DialResult()

    /** The desktop declined: bad token, bad HMAC, or a replayed nonce. */
    data object Denied : DialResult()

    /**
     * The PC does not recognise this phone's stored credentials.
     *
     * Distinct from [Denied] because the remedy is different: nothing was
     * declined, the PC simply has no record of this device, so the phone has to
     * pair again rather than wait for an approval that will never come.
     */
    data object CredentialsRejected : DialResult()

    data object VersionMismatch : DialResult()

    data object Busy : DialResult()

    data object TokenExpired : DialResult()

    data object BadRequest : DialResult()

    data object UnauthorizedSource : DialResult()

    data class Unreachable(val message: String) : DialResult()

    data class Failed(val message: String) : DialResult()
}

/**
 * Maps the desktop's handshake status to the outcome the session acts on.
 *
 * Kept pure and socket-free — [connection] is only evaluated for the two
 * outcomes that keep the channel — so the decision that says "pair again" or
 * "here is an error" is unit-testable. The mapping is where the phone's recovery
 * from a PC that no longer knows it was silently broken, and a live dial is not
 * available to a unit test, but the decision that was wrong is.
 *
 * The subtle case is [HandshakeStatus.DENIED]. A human *denial* never arrives as
 * this status: the desktop answers a pair request with
 * [HandshakeStatus.APPROVAL_PENDING] and then pushes the user's decision down
 * that same socket as a `pair_denied` control message, after which it closes the
 * socket instead of promoting it to the control channel
 * (`CoreSession::HandlePhoneConnection`, `native/core/src/core_session.cpp`). So
 * on a device-secret dial `denied` cannot mean "a person said no" — the core
 * sends it only when it has no record of the device, when the HMAC does not
 * match, or when the nonce replays (`CoreSession::Authenticate`). Each of those
 * means the stored secret is no longer usable, and pairing again is the way out,
 * so a device-secret `denied` becomes [DialResult.CredentialsRejected]. A
 * pairing-token dial is different: the token is a freshly scanned QR code rather
 * than a stale secret, `denied` there is a genuine refusal, so it stays
 * [DialResult.Denied].
 *
 * [HandshakeStatus.TOKEN_EXPIRED] is [DialResult.CredentialsRejected] on both
 * modes: the QR path reports an unknown or expired token this way, and there is
 * nothing to wait for.
 */
fun dialResultFor(
    status: HandshakeStatus,
    authMode: AuthMode,
    connection: () -> PeerConnection,
): DialResult = when (status) {
    HandshakeStatus.OK -> DialResult.Ok(connection())
    HandshakeStatus.APPROVAL_PENDING -> DialResult.ApprovalPending(connection())
    HandshakeStatus.DENIED ->
        if (authMode == AuthMode.DEVICE_SECRET) DialResult.CredentialsRejected else DialResult.Denied
    HandshakeStatus.VERSION_MISMATCH -> DialResult.VersionMismatch
    HandshakeStatus.BUSY -> DialResult.Busy
    HandshakeStatus.TOKEN_EXPIRED -> DialResult.CredentialsRejected
    HandshakeStatus.BAD_REQUEST -> DialResult.BadRequest
    HandshakeStatus.UNAUTHORIZED_SOURCE -> DialResult.UnauthorizedSource
}

/** A dialled, handshake-completed TCP connection with framed readers/writers. */
class PeerConnection(
    val socket: Socket,
    val input: DataInputStream,
    val output: DataOutputStream,
    val role: ChannelRole,
    val response: ServerResponse,
) : AutoCloseable {

    @Volatile
    private var closed = false

    override fun close() {
        if (closed) return
        closed = true
        runCatching { socket.close() }
    }

    val isOpen: Boolean
        get() = !closed && socket.isConnected && !socket.isClosed

    companion object {
        private const val TAG = "PeerConnection"
        private const val SOCKET_BUFFER = 256 * 1024
        private const val CONNECT_TIMEOUT_MS = 4000
        private const val HANDSHAKE_TIMEOUT_MS = 8000

        /**
         * Dials the desktop and completes the handshake.
         *
         * The phone is always the TCP client (docs/protocol.md §1). Several
         * credentials sign the desktop's fresh nonce, so [buildAuth] receives the
         * greeting's nonce rather than the caller passing a finished string.
         */
        fun dial(
            host: String,
            port: Int,
            role: ChannelRole,
            authMode: AuthMode,
            clientNonce: Long,
            buildAuth: (serverNonce: ByteArray) -> String = { "" },
            connectTimeoutMs: Int = CONNECT_TIMEOUT_MS,
            handshakeTimeoutMs: Int = HANDSHAKE_TIMEOUT_MS,
        ): DialResult {
            val socket = Socket()
            return try {
                socket.tcpNoDelay = true
                socket.connect(InetSocketAddress(host, port), connectTimeoutMs)
                socket.soTimeout = handshakeTimeoutMs

                val input = DataInputStream(BufferedInputStream(socket.getInputStream(), SOCKET_BUFFER))
                val output = DataOutputStream(BufferedOutputStream(socket.getOutputStream(), SOCKET_BUFFER))

                val greeting = Framing.readGreeting(input)
                if (greeting.protoVersion != Proto.VERSION) {
                    socket.close()
                    return DialResult.VersionMismatch
                }
                if (greeting.status != HandshakeStatus.OK) {
                    socket.close()
                    return when (greeting.status) {
                        HandshakeStatus.BUSY -> DialResult.Busy
                        HandshakeStatus.VERSION_MISMATCH -> DialResult.VersionMismatch
                        else -> DialResult.Failed("The PC refused the connection.")
                    }
                }

                Framing.writeHandshake(output, role, authMode, clientNonce, buildAuth(greeting.serverNonce))
                val response = Framing.readResponse(input)

                // Past the handshake the channel is long-lived and must not time
                // out while the phone waits idle for a control command.
                socket.soTimeout = 0

                val connection = PeerConnection(socket, input, output, role, response)
                val mapped = dialResultFor(response.status, authMode) { connection }
                // Only the two outcomes that keep the channel may hold the
                // socket; every refusal is closed before it is reported.
                if (mapped !is DialResult.Ok && mapped !is DialResult.ApprovalPending) {
                    connection.close()
                }
                mapped
            } catch (timeout: SocketTimeoutException) {
                runCatching { socket.close() }
                DialResult.Unreachable("The PC didn't respond. Check that PhoneCam Desktop is running.")
            } catch (io: IOException) {
                runCatching { socket.close() }
                Log.w(TAG, "dial to $host:$port failed: ${io.javaClass.simpleName}")
                DialResult.Unreachable(
                    "Couldn't reach $host:$port. Check the address and that both devices are on the same network.",
                )
            } catch (t: Throwable) {
                runCatching { socket.close() }
                Log.w(TAG, "dial to $host:$port threw ${t.javaClass.simpleName}")
                DialResult.Failed("Connection failed: ${t.javaClass.simpleName}")
            }
        }

        /**
         * Dials the video channel as a device the desktop already knows.
         *
         * `StreamSession` calls this to open `role = 2`. It is [dialWithTokenOrSecret]
         * without the token branch, and it is kept as a separate name because the
         * video channel is never the place a pairing starts: a token can only be
         * exchanged over the control channel (docs/protocol.md §4.2).
         */
        fun dialAsKnownDevice(
            identity: IdentityStore,
            host: String,
            port: Int,
            role: ChannelRole,
        ): DialResult = dialWithTokenOrSecret(
            identity = identity,
            host = host,
            port = port,
            role = role,
            token = null,
        )

        /**
         * Dials with either a scanned pairing token or the stored device secret.
         *
         * This is the single entry point the session uses for the control channel:
         * QR pairing when a token is present, silent re-authentication when the
         * phone already holds a secret, and a desktop approval prompt otherwise
         * (docs §4.2, §4.3, §4.4).
         */
        fun dialWithTokenOrSecret(
            identity: IdentityStore,
            host: String,
            port: Int,
            role: ChannelRole,
            token: String? = null,
        ): DialResult {
            val clientNonce = identity.nextClientNonce()

            if (token != null) {
                return dial(
                    host = host,
                    port = port,
                    role = role,
                    authMode = AuthMode.PAIRING_TOKEN,
                    clientNonce = clientNonce,
                    buildAuth = { token },
                )
            }

            val secret = identity.deviceSecretHex
            return if (secret != null) {
                dial(
                    host = host,
                    port = port,
                    role = role,
                    authMode = AuthMode.DEVICE_SECRET,
                    clientNonce = clientNonce,
                    buildAuth = { serverNonce ->
                        Auth.deviceSecretAuth(identity.deviceId, secret, serverNonce, clientNonce)
                    },
                )
            } else {
                dial(
                    host = host,
                    port = port,
                    role = role,
                    authMode = AuthMode.PAIR_REQUEST,
                    clientNonce = clientNonce,
                    buildAuth = { serverNonce ->
                        Auth.pairRequestAuth(identity.deviceId, null, serverNonce, clientNonce)
                    },
                )
            }
        }
    }
}
