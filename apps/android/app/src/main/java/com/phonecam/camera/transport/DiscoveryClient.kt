package com.phonecam.camera.transport

import android.content.Context
import android.net.wifi.WifiManager
import android.util.Log
import com.phonecam.camera.nativecore.NativeCore
import com.phonecam.camera.protocol.ControlCodec
import com.phonecam.camera.protocol.Proto
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.InterfaceAddress
import java.net.NetworkInterface

/** A desktop found on the local network (PRD §9). */
data class DiscoveredServer(
    val serverId: String,
    val name: String,
    val host: String,
    val port: Int,
    val pairingOpen: Boolean,
    val knowsThisDevice: Boolean,
    val busy: Boolean,
    val lastSeenMs: Long,
)

/**
 * UDP discovery on [Proto.DISCOVERY_PORT] (docs/protocol.md §3).
 *
 * Discovery is a convenience, never the only path: QR pairing and manual IP both
 * work with discovery disabled, which is what PRD §9 requires as a fallback. A
 * failure here is therefore logged and swallowed rather than surfaced as an
 * error, because the user can always connect without it.
 */
class DiscoveryClient(
    context: Context,
    private val scope: CoroutineScope,
    private val deviceId: String,
    private val beaconProvider: () -> JsonObject,
) {

    private val wifiManager =
        context.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager

    private val _servers = MutableSharedFlow<List<DiscoveredServer>>(replay = 1, extraBufferCapacity = 8)
    val servers: SharedFlow<List<DiscoveredServer>> = _servers

    /**
     * The one discovery socket, used for sending *and* receiving.
     *
     * A probe has to leave from the port its answer is expected on: §3.3 has the
     * core reply **unicast to the sender's address and port**, so a probe sent
     * from an ephemeral socket is answered to a port nothing reads and discovery
     * never completes — the listening socket stays silent while the reply is
     * dropped on the floor.
     */
    @Volatile
    private var receiveSocket: DatagramSocket? = null
    private var receiveJob: Job? = null
    private var beaconJob: Job? = null

    /**
     * Whether the periodic beacon is currently wanted.
     *
     * Set by [setBeaconing] and read by [beaconLoop]. A plain `Boolean` is
     * enough: the loop re-reads it on every tick, so a write that lands mid-tick
     * is seen on the next one at worst.
     */
    @Volatile
    private var beaconing = false

    private val known = LinkedHashMap<String, DiscoveredServer>()

    /**
     * Every address a given desktop has advertised, not just the latest one.
     *
     * A PC with Wi-Fi and Ethernet sends one beacon per interface, and the phone
     * may be able to reach one address and not the other. `known` keeps one entry
     * per server (the UI must not list the same PC twice), so without this map the
     * earlier address was lost the moment the later beacon arrived, and a
     * reconnect could never try the other one.
     */
    private val hostsByServer = LinkedHashMap<String, LinkedHashSet<String>>()

    /** Addresses seen for [serverId], newest first is not meaningful here. */
    fun hostsFor(serverId: String?): List<String> {
        if (serverId != null) return hostsByServer[serverId]?.toList().orEmpty()
        return hostsByServer.values.flatten().distinct()
    }

    fun start() {
        if (receiveJob?.isActive == true) return
        // Receive only. Beaconing is a separate decision ([setBeaconing]): the
        // phone must be able to hear a search probe from the desktop even when it
        // is not advertising itself, and it must be able to advertise itself
        // while the user is looking for a PC.
        receiveJob = scope.launch(Dispatchers.IO) { receiveLoop() }
    }

    /**
     * Turns the periodic beacon on or off.
     *
     * Off is the right state whenever the phone already has a desktop: a
     * connection is already established, so a 1 Hz broadcast to every interface
     * has no one left to inform. The receive loop stays up either way, so the
     * phone still answers a `phonecam.probe` and still sees a desktop beacon.
     */
    fun setBeaconing(enabled: Boolean) {
        beaconing = enabled
        if (enabled) {
            if (beaconJob?.isActive == true) return
            beaconJob = scope.launch(Dispatchers.IO) { beaconLoop() }
        } else {
            beaconJob?.cancel()
            beaconJob = null
        }
    }

    fun stop() {
        beaconing = false
        receiveJob?.cancel()
        beaconJob?.cancel()
        receiveJob = null
        beaconJob = null
        runCatching { receiveSocket?.close() }
        receiveSocket = null
        known.clear()
        hostsByServer.clear()
    }

    /** Sends a probe so a desktop answers immediately rather than waiting (§3.3). */
    fun probe() {
        scope.launch(Dispatchers.IO) {
            val payload = ControlCodec.encodeDiscovery(
                buildJsonObject {
                    put("t", "phonecam.probe")
                    put("v", Proto.VERSION)
                    put("role", "phone")
                },
            )
            // The receive loop binds the socket on its own coroutine, so a probe
            // fired at a freshly started session would otherwise be dropped — and
            // a probe sent from any other port is answered to a port nothing
            // reads. Waiting for the socket is bounded and shorter than the
            // window the user is watching.
            awaitReceiveSocket() ?: return@launch
            broadcast(payload)
        }
    }

    private suspend fun beaconLoop() {
        // The first tick races the receive loop's bind; waiting for it means the
        // very first beacon is sent rather than dropped.
        awaitReceiveSocket()
        while (scope.isActive) {
            // Re-checked every tick, so turning beaconing off stops the broadcast
            // without having to cancel this coroutine mid-sleep.
            if (beaconing) {
                runCatching { broadcast(ControlCodec.encodeDiscovery(beaconProvider())) }
                    .onFailure { Log.d(TAG, "beacon send failed: ${it.javaClass.simpleName}") }
            }
            delay(Proto.BEACON_INTERVAL_MS)
        }
    }

    /**
     * Listens for beacons and probes and answers the latter.
     *
     * This runs for the whole session, not only while the user is searching: the
     * desktop may probe a phone that is already connected (it re-runs discovery
     * when its USB state changes), and a reply has to leave from this socket's
     * port, so closing it while connected would make the phone unreachable to
     * that probe. What the connection removes is the *beacon*, not the listener
     * (see [setBeaconing]).
     *
     * The multicast lock is held for exactly as long as this loop runs, and this
     * loop is the only thing that needs it: the Wi-Fi stack drops broadcast
     * datagrams to save power, which is what the lock prevents. Releasing it here
     * — in the `finally`, when the loop stops — keeps it from outliving the
     * receive path it exists for.
     */
    private suspend fun receiveLoop() {
        val lock: WifiManager.MulticastLock? = runCatching {
            wifiManager?.createMulticastLock("phonecam-discovery")?.apply {
                setReferenceCounted(false)
                acquire()
            }
        }.getOrNull()

        try {
            val socket = DatagramSocket(null).apply {
                reuseAddress = true
                broadcast = true
                bind(InetSocketAddress(Proto.DISCOVERY_PORT))
            }
            receiveSocket = socket
            val buffer = ByteArray(4096)
            while (scope.isActive) {
                val packet = DatagramPacket(buffer, buffer.size)
                try {
                    socket.receive(packet)
                } catch (closed: Exception) {
                    if (!scope.isActive) return
                    continue
                }
                runCatching { handlePacket(packet) }
                    .onFailure { Log.d(TAG, "beacon parse failed: ${it.javaClass.simpleName}") }
            }
        } catch (t: Throwable) {
            // Discovery failing must never break the app: QR and manual IP remain.
            Log.w(TAG, "discovery listener unavailable: ${t.javaClass.simpleName}")
        } finally {
            runCatching { lock?.release() }
        }
    }

    private suspend fun handlePacket(packet: DatagramPacket) {
        val rawBytes = packet.data.copyOfRange(packet.offset, packet.offset + packet.length)
        // Strip the IPv6 scope suffix so the value is dialable.
        val sourceHost = packet.address?.hostAddress?.substringBefore('%')

        val beacon = parseDesktopBeacon(
            jsonBytes = rawBytes,
            sourceHost = sourceHost,
            deviceId = deviceId,
        )
        if (beacon != null) {
            known[beacon.serverId] = beacon
            hostsByServer.getOrPut(beacon.serverId) { LinkedHashSet() }.add(beacon.host)
            publish()
            return
        }

        val text = String(rawBytes, Charsets.UTF_8)
        val obj = runCatching { Json.parseToJsonElement(text) as? JsonObject }.getOrNull() ?: return
        val type = obj.string("t") ?: return
        if (obj.string("v")?.toIntOrNull() != Proto.VERSION) return

        if (type == "phonecam.probe") {
            // Answer unicast so the desktop learns about this phone (§3.3).
            // The same socket answers, which is what keeps the phone's source
            // port stable across the exchange.
            val payload = ControlCodec.encodeDiscovery(beaconProvider())
            val socket = receiveSocket ?: return
            runCatching {
                socket.send(DatagramPacket(payload, payload.size, packet.address, packet.port))
            }
        }
    }

    private suspend fun publish() {
        val now = System.currentTimeMillis()
        known.entries.removeAll { now - it.value.lastSeenMs > SERVER_TTL_MS }
        // Drop addresses for servers that are no longer being seen, so a stale
        // address cannot be offered as a fallback after the PC has gone.
        hostsByServer.keys.retainAll(known.keys)
        _servers.emit(known.values.sortedByDescending { it.lastSeenMs })
    }

    /**
     * Sends to the limited broadcast address and to every interface's directed
     * broadcast address, because routers and some Wi-Fi drivers drop one or the
     * other. The USB-tethered interface is included by the same loop.
     *
     * Sends through [receiveSocket] so the packet's source port is the one the
     * phone is listening on: §3.3 answers a probe unicast *to the sender's port*,
     * so a probe sent from anywhere else is answered into a void. When the socket
     * is not bound yet the send is skipped rather than thrown — the periodic
     * beacon retries a second later, and the receive loop is what makes discovery
     * possible at all.
     */
    private fun broadcast(payload: ByteArray) {
        val socket = receiveSocket ?: return
        val targets = linkedSetOf<InetAddress>()
        runCatching { targets += InetAddress.getByName("255.255.255.255") }
        runCatching {
            NetworkInterface.getNetworkInterfaces()?.toList()?.forEach { nif ->
                if (!nif.isUp || nif.isLoopback) return@forEach
                nif.interfaceAddresses.forEach { address: InterfaceAddress ->
                    address.broadcast?.let { targets += it }
                }
            }
        }
        targets.forEach { target ->
            runCatching {
                socket.send(DatagramPacket(payload, payload.size, target, Proto.DISCOVERY_PORT))
            }
        }
    }

    /**
     * Waits for [receiveLoop] to bind the socket, up to [BIND_WAIT_MS].
     *
     * Bounded because the loop can fail to bind outright (another app holds the
     * port, or the network is down); in that case the caller gives up quietly,
     * which is the same "discovery is a convenience" contract the class already
     * follows everywhere else.
     */
    private suspend fun awaitReceiveSocket(): DatagramSocket? {
        val deadline = System.currentTimeMillis() + BIND_WAIT_MS
        while (receiveSocket == null && scope.isActive) {
            if (System.currentTimeMillis() >= deadline) return null
            delay(BIND_POLL_MS)
        }
        return receiveSocket
    }

    private fun JsonObject.string(key: String): String? =
        (this[key] as? JsonPrimitive)?.contentOrNull()

    private fun JsonPrimitive.contentOrNull(): String? =
        runCatching { content }.getOrNull()

    @Serializable
    private data class NormalizedDesktopBeacon(
        val t: String,
        val v: Int,
        val role: String,
        val serverId: String,
        val name: String = "PhoneCam Desktop",
        val host: String = "",
        val port: Int = Proto.SERVICE_PORT,
        val pairingOpen: Boolean = false,
        val pairingId: String? = null,
        val pairedDeviceIds: List<String> = emptyList(),
        val busy: Boolean = false,
        val version: String = "",
    )

    companion object {
        private const val TAG = "DiscoveryClient"
        private const val SERVER_TTL_MS = 5000L

        /** How long a probe waits for the receive socket to bind. */
        private const val BIND_WAIT_MS = 1000L
        private const val BIND_POLL_MS = 20L

        private val beaconJson = Json { ignoreUnknownKeys = true }

        /**
         * Validates and parses a raw UDP discovery beacon via the Rust core.
         * Returns `null` for probes, phone beacons, or malformed packets.
         */
        fun parseDesktopBeacon(
            jsonBytes: ByteArray,
            sourceHost: String?,
            deviceId: String,
            nowMs: Long = System.currentTimeMillis(),
        ): DiscoveredServer? {
            val normalized = NativeCore.parseDiscoveryBeacon(jsonBytes) ?: return null
            val parsed = runCatching {
                beaconJson.decodeFromString(NormalizedDesktopBeacon.serializer(), normalized)
            }.getOrNull() ?: return null
            val resolvedHost = sourceHost?.takeIf { it.isNotBlank() }
                ?: parsed.host.takeIf { it.isNotBlank() }
                ?: return null
            return DiscoveredServer(
                serverId = parsed.serverId,
                name = parsed.name.ifBlank { "PhoneCam Desktop" },
                host = resolvedHost,
                port = parsed.port,
                pairingOpen = parsed.pairingOpen,
                knowsThisDevice = parsed.pairedDeviceIds.contains(deviceId),
                busy = parsed.busy,
                lastSeenMs = nowMs,
            )
        }
    }
}
