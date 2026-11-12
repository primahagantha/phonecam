package com.phonecam.camera.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.phonecam.camera.R
import com.phonecam.camera.protocol.Proto
import com.phonecam.camera.session.SearchState
import com.phonecam.camera.session.SessionState
import com.phonecam.camera.transport.DiscoveredServer

/**
 * Everything about reaching the PC: the three connection paths from PRD §9, in
 * the order the product prefers them.
 *
 *  1. QR pairing — the action the app leads with;
 *  2. discovery — the list of PCs that answered a beacon;
 *  3. manual address — the fallback that must work when discovery is blocked by
 *     the network, which is common on guest Wi-Fi and with client isolation.
 *
 * The card has one primary action and one secondary, and the secondary ones are
 * nested inside the block they belong to rather than lined up beside the primary.
 * That is the difference between a screen with one obvious next step and a screen
 * with five buttons of equal weight.
 *
 * Starting and stopping the session deliberately does **not** live here. That
 * action has to be reachable from every screen, so it sits in the persistent
 * status strip above the navigation bar, and the card covers only how the phone
 * reaches the PC.
 */
@Composable
fun ConnectionCard(
    state: SessionState,
    isRunning: Boolean,
    pairedServerName: String?,
    pairedAddress: String?,
    onScanQr: () -> Unit,
    onSearch: () -> Unit,
    onConnectDiscovered: (DiscoveredServer) -> Unit,
    onConnectManual: (String, Int) -> Unit,
    onForget: () -> Unit,
    modifier: Modifier = Modifier,
) {
    SectionCard(
        title = stringResource(R.string.section_connection),
        modifier = modifier,
    ) {
        PairingBlock(
            isRunning = isRunning,
            pairedServerName = pairedServerName,
            pairedAddress = pairedAddress,
            onScanQr = onScanQr,
            onForget = onForget,
        )

        Hairline()
        DiscoveryBlock(
            state = state,
            onSearch = onSearch,
            onConnectDiscovered = onConnectDiscovered,
        )

        Hairline()
        ManualAddressRow(onConnect = onConnectManual)

        // Only while the phone is looking for a PC: once one is connected, a
        // developer-options tutorial is noise. The cable path is the manual
        // address 127.0.0.1, so the hint sits directly under it.
        if (state.phase == SessionState.Phase.IDLE || state.phase == SessionState.Phase.DISCOVERING) {
            Spacer(Modifier.height(8.dp))
            Text(
                text = stringResource(R.string.usb_hint_title) + "\n" +
                    stringResource(R.string.usb_hint_body),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/**
 * The paired PC, and the two things that can be done to it.
 *
 * When there is no pairing this is an empty state with the action that fills it,
 * rather than a row of values reading "None" under a "Forget" button that would
 * have nothing to forget.
 */
@Composable
private fun PairingBlock(
    isRunning: Boolean,
    pairedServerName: String?,
    pairedAddress: String?,
    onScanQr: () -> Unit,
    onForget: () -> Unit,
) {
    SectionLabel(stringResource(R.string.paired_title))
    if (pairedServerName == null) {
        EmptyState(stringResource(R.string.paired_none))
        OutlinedButton(onClick = onScanQr) {
            Text(stringResource(R.string.action_scan_qr))
        }
        return
    }

    LabeledValue(label = stringResource(R.string.label_pc), value = pairedServerName)
    if (pairedAddress != null) {
        LabeledValue(label = stringResource(R.string.label_address), value = pairedAddress)
    }
    if (isRunning) {
        Hint(stringResource(R.string.stream_start_hint))
    }
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedButton(onClick = onScanQr) {
            Text(stringResource(R.string.action_scan_qr))
        }
        TextButton(onClick = onForget) {
            Text(stringResource(R.string.action_forget_pairing))
        }
    }
}

/**
 * The list of PCs that answered a beacon, with its own search control.
 *
 * The three states a search can be in are all rendered here and are all
 * distinguishable:
 *
 *  - *loading*: the button is disabled and a spinner sits beside it, so the
 *    window where a second tap would restart the timer is closed;
 *  - *empty*: prose that names the likely cause and points at the pairing code;
 *  - *found*: one row per PC, each with its own Connect.
 *
 * The outcome is announced as a polite live region because it is not visible
 * anywhere else: a list that is empty looks the same whether nothing has been
 * tried or nothing answered, and a PC that replies only shows up as a new row.
 */
@Composable
private fun DiscoveryBlock(
    state: SessionState,
    onSearch: () -> Unit,
    onConnectDiscovered: (DiscoveredServer) -> Unit,
) {
    SectionLabel(stringResource(R.string.discovered_title))

    when (val search = state.search) {
        SearchState.Searching -> {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                CircularProgressIndicator(
                    modifier = Modifier.size(16.dp),
                    strokeWidth = 2.dp,
                )
                Text(
                    text = stringResource(R.string.search_status_searching),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier
                        .weight(1f)
                        .semantics { liveRegion = LiveRegionMode.Polite },
                )
            }
        }

        is SearchState.Found -> {
            Text(
                text = pluralStringResource(
                    R.plurals.search_status_found,
                    search.count,
                    search.count,
                ),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
            )
        }

        SearchState.NotFound -> {
            Text(
                text = stringResource(R.string.search_status_not_found),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
            )
        }

        SearchState.Idle -> Unit
    }

    if (state.discovered.isEmpty()) {
        EmptyState(
            stringResource(
                // "Nothing answered" needs the user to act; "not tried yet" does
                // not, and saying "No PC found yet" for both would report a
                // finished search as an unstarted one.
                if (state.search == SearchState.NotFound) {
                    R.string.discovered_empty_not_found
                } else {
                    R.string.discovered_empty
                },
            ),
        )
    } else {
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            state.discovered.forEach { server ->
                DiscoveredServerRow(
                    server = server,
                    onConnect = { onConnectDiscovered(server) },
                )
            }
        }
    }

    TextButton(
        onClick = onSearch,
        // A second tap during the window would restart the timer and extend the
        // spinner, so the button is unavailable until the search it started has
        // finished.
        enabled = state.search != SearchState.Searching,
    ) {
        Text(stringResource(R.string.action_search_now))
    }
}

@Composable
private fun DiscoveredServerRow(server: DiscoveredServer, onConnect: () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = server.name,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurface,
                modifier = Modifier.weight(1f),
            )
            OutlinedButton(onClick = onConnect) {
                Text(stringResource(R.string.action_connect))
            }
        }
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = "${server.host}:${server.port}",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.weight(1f),
            )
            // The state is a token rather than a suffix on the address, because
            // it is the thing that decides whether Connect will work. The token's
            // colour comes from the same rule as its words, so the visual cannot
            // imply more than the label says.
            StatusToken(
                text = discoveredStateLabel(server),
                state = discoveredTokenState(server),
            )
        }
    }
}

/**
 * What a discovery beacon establishes about a PC, from the flags it set.
 *
 * The flags are what the PC advertised, not a promise about connecting: a beacon
 * that set none of them (not busy, does not know this phone, not advertising a
 * pairing window) still only establishes that the PC is there. The old fallback
 * label called that "Ready", which claimed a connectability the beacon does not
 * establish — connecting still requires pairing. This classifier is the single
 * source for both the label and the token's colour so the two cannot drift.
 */
internal enum class DiscoveredAvailability {
    /** Occupied with another phone. */
    Busy,

    /** The PC already knows this phone. */
    Known,

    /** The PC is advertising a pairing window. */
    PairingOpen,

    /** The PC is there, but its beacon established nothing more. */
    Present,
}

internal fun discoveredAvailability(server: DiscoveredServer): DiscoveredAvailability = when {
    server.busy -> DiscoveredAvailability.Busy
    server.knowsThisDevice -> DiscoveredAvailability.Known
    server.pairingOpen -> DiscoveredAvailability.PairingOpen
    else -> DiscoveredAvailability.Present
}

@Composable
private fun discoveredStateLabel(server: DiscoveredServer): String = stringResource(
    when (discoveredAvailability(server)) {
        DiscoveredAvailability.Busy -> R.string.discovered_state_busy
        DiscoveredAvailability.Known -> R.string.discovered_state_known
        DiscoveredAvailability.PairingOpen -> R.string.discovered_state_pairing
        // Not "Ready": the beacon only says the PC is here.
        DiscoveredAvailability.Present -> R.string.discovered_state_present
    },
)

/**
 * The token state for a discovered PC.
 *
 * Only the cases that establish something get a coloured state; [Present] is
 * [TokenState.Neutral] so the token does not read "Available" for a PC whose
 * beacon said nothing beyond its presence.
 */
private fun discoveredTokenState(server: DiscoveredServer): TokenState = when (discoveredAvailability(server)) {
    DiscoveredAvailability.Busy -> TokenState.Unavailable
    DiscoveredAvailability.Present -> TokenState.Neutral
    DiscoveredAvailability.Known,
    DiscoveredAvailability.PairingOpen,
    -> TokenState.Available
}

/**
 * The fallback path, kept open but not advertised.
 *
 * It is collapsed by default and sits at the bottom of the card behind its own
 * toggle. Two reasons: it is the answer to a specific failure ("discovery is
 * blocked") rather than a general way to connect, and a permanently visible pair
 * of empty text fields reads as something the user is expected to fill in, which
 * competes with the primary action above it. Opening it is remembered across
 * rotation so a half-typed address is not thrown away.
 *
 * Validation is inline and names the fix, never the rule.
 */
@Composable
private fun ManualAddressRow(onConnect: (String, Int) -> Unit) {
    var expanded by rememberSaveable { mutableStateOf(false) }
    var host by rememberSaveable { mutableStateOf("") }
    var port by rememberSaveable { mutableStateOf(Proto.SERVICE_PORT.toString()) }
    var problem by rememberSaveable { mutableStateOf<Int?>(null) }

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        TextButton(onClick = { expanded = !expanded }) {
            Text(
                stringResource(
                    if (expanded) R.string.action_manual_hide else R.string.action_manual_show,
                ),
            )
        }
        if (!expanded) return@Column

        SectionLabel(stringResource(R.string.manual_title))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(
                value = host,
                onValueChange = {
                    host = it
                    problem = null
                },
                label = { Text(stringResource(R.string.manual_host_label)) },
                placeholder = { Text(stringResource(R.string.manual_host_hint)) },
                singleLine = true,
                isError = problem == R.string.error_invalid_host,
                modifier = Modifier.weight(1.6f),
            )
            OutlinedTextField(
                value = port,
                onValueChange = {
                    port = it
                    problem = null
                },
                label = { Text(stringResource(R.string.manual_port_label)) },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                isError = problem == R.string.error_invalid_port,
                modifier = Modifier.weight(1f),
            )
        }
        problem?.let { message ->
            Text(
                text = stringResource(message),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
            )
        }
        Hint(stringResource(R.string.manual_hint))
        OutlinedButton(
            onClick = {
                val cleanedHost = host.trim()
                val cleanedPort = port.trim().toIntOrNull()
                when {
                    cleanedHost.isEmpty() -> problem = R.string.error_invalid_host
                    cleanedPort == null || cleanedPort !in 1..65535 -> problem = R.string.error_invalid_port
                    else -> onConnect(cleanedHost, cleanedPort)
                }
            },
        ) {
            Text(stringResource(R.string.action_connect))
        }
    }
}
