package com.phonecam.camera.ui.components

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.phonecam.camera.R
import com.phonecam.camera.protocol.Proto
import com.phonecam.camera.session.SearchState
import com.phonecam.camera.session.SessionState
import com.phonecam.camera.transport.DiscoveredServer
import com.phonecam.camera.ui.theme.PhoneCamShapes
import com.phonecam.camera.ui.theme.PhoneCamStudioTheme

/**
 * Connection card: Paired PC, LAN Discovery, and Manual Wi-Fi/USB IP fallback.
 *
 * Uses crisp Studio Blue primary CTA for first-contact pairing, elevated inset
 * rows for discovered servers, and subtle danger tint for forgetting a paired PC.
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
        subtitle = stringResource(R.string.section_connection_subtitle),
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

        if (state.phase == SessionState.Phase.IDLE || state.phase == SessionState.Phase.DISCOVERING) {
            Spacer(Modifier.height(2.dp))
            InsetWell {
                Text(
                    text = stringResource(R.string.usb_hint_title),
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.onSurface,
                )
                Text(
                    text = stringResource(R.string.usb_hint_body),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun PairingBlock(
    isRunning: Boolean,
    pairedServerName: String?,
    pairedAddress: String?,
    onScanQr: () -> Unit,
    onForget: () -> Unit,
) {
    val studio = PhoneCamStudioTheme.colors
    SectionLabel(stringResource(R.string.paired_title))
    if (pairedServerName == null) {
        EmptyState(stringResource(R.string.paired_none))
        Button(
            onClick = onScanQr,
            shape = PhoneCamShapes.Field,
            contentPadding = PaddingValues(horizontal = 16.dp, vertical = 10.dp),
        ) {
            Icon(
                imageVector = Icons.Filled.QrCodeScanner,
                contentDescription = null,
                modifier = Modifier.size(18.dp),
            )
            Spacer(Modifier.width(8.dp))
            Text(
                text = stringResource(R.string.action_scan_qr),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        return
    }

    InsetWell {
        LabeledValue(label = stringResource(R.string.label_pc), value = pairedServerName)
        if (pairedAddress != null) {
            LabeledValue(label = stringResource(R.string.label_address), value = pairedAddress)
        }
    }
    if (isRunning) {
        Hint(stringResource(R.string.stream_start_hint))
    }
    FlowRow(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        OutlinedButton(
            onClick = onScanQr,
            shape = PhoneCamShapes.Chip,
            border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
            contentPadding = PaddingValues(horizontal = 14.dp, vertical = 8.dp),
        ) {
            Icon(
                imageVector = Icons.Filled.QrCodeScanner,
                contentDescription = null,
                modifier = Modifier.size(16.dp),
            )
            Spacer(Modifier.width(6.dp))
            Text(
                text = stringResource(R.string.action_scan_qr),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        TextButton(
            onClick = onForget,
            shape = PhoneCamShapes.Chip,
            colors = ButtonDefaults.textButtonColors(
                contentColor = studio.dangerText,
            ),
            contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
        ) {
            Text(
                text = stringResource(R.string.action_forget_pairing),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
private fun DiscoveryBlock(
    state: SessionState,
    onSearch: () -> Unit,
    onConnectDiscovered: (DiscoveredServer) -> Unit,
) {
    val studio = PhoneCamStudioTheme.colors
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        SectionLabel(stringResource(R.string.discovered_title))
        TextButton(
            onClick = onSearch,
            enabled = state.search != SearchState.Searching,
            shape = PhoneCamShapes.Chip,
            contentPadding = PaddingValues(horizontal = 10.dp, vertical = 4.dp),
        ) {
            Icon(
                imageVector = Icons.Filled.Refresh,
                contentDescription = null,
                modifier = Modifier.size(15.dp),
            )
            Spacer(Modifier.width(5.dp))
            Text(
                text = stringResource(R.string.action_search_now),
                style = MaterialTheme.typography.labelLarge,
                maxLines = 1,
            )
        }
    }

    when (val search = state.search) {
        SearchState.Searching -> {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                CircularProgressIndicator(
                    modifier = Modifier.size(15.dp),
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
                color = studio.live,
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
                if (state.search == SearchState.NotFound) {
                    R.string.discovered_empty_not_found
                } else {
                    R.string.discovered_empty
                },
            ),
        )
    } else {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            state.discovered.forEach { server ->
                DiscoveredServerRow(
                    server = server,
                    onConnect = { onConnectDiscovered(server) },
                )
            }
        }
    }
}

@Composable
private fun DiscoveredServerRow(server: DiscoveredServer, onConnect: () -> Unit) {
    InsetWell {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Text(
                    text = server.name,
                    style = MaterialTheme.typography.titleSmall,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = "${server.host}:${server.port}",
                    style = MaterialTheme.typography.bodySmall.copy(fontFeatureSettings = "tnum"),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Button(
                onClick = onConnect,
                shape = PhoneCamShapes.Chip,
                contentPadding = PaddingValues(horizontal = 14.dp, vertical = 7.dp),
            ) {
                Text(
                    text = stringResource(R.string.action_connect),
                    style = MaterialTheme.typography.labelLarge,
                    maxLines = 1,
                )
            }
        }
        StatusToken(
            text = discoveredStateLabel(server),
            state = discoveredTokenState(server),
        )
    }
}

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
        DiscoveredAvailability.Present -> R.string.discovered_state_present
    },
)

private fun discoveredTokenState(server: DiscoveredServer): TokenState = when (discoveredAvailability(server)) {
    DiscoveredAvailability.Busy -> TokenState.Unavailable
    DiscoveredAvailability.Present -> TokenState.Neutral
    DiscoveredAvailability.Known,
    DiscoveredAvailability.PairingOpen,
    -> TokenState.Available
}

@Composable
private fun ManualAddressRow(onConnect: (String, Int) -> Unit) {
    var expanded by rememberSaveable { mutableStateOf(false) }
    var host by rememberSaveable { mutableStateOf("") }
    var port by rememberSaveable { mutableStateOf(Proto.SERVICE_PORT.toString()) }
    var problem by rememberSaveable { mutableStateOf<Int?>(null) }

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        TextButton(
            onClick = { expanded = !expanded },
            shape = PhoneCamShapes.Chip,
            contentPadding = PaddingValues(horizontal = 4.dp, vertical = 4.dp),
        ) {
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
                shape = PhoneCamShapes.Field,
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
                shape = PhoneCamShapes.Field,
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
        Button(
            onClick = {
                val cleanedHost = host.trim()
                val cleanedPort = port.trim().toIntOrNull()
                when {
                    cleanedHost.isEmpty() -> problem = R.string.error_invalid_host
                    cleanedPort == null || cleanedPort !in 1..65535 -> problem = R.string.error_invalid_port
                    else -> onConnect(cleanedHost, cleanedPort)
                }
            },
            shape = PhoneCamShapes.Chip,
            contentPadding = PaddingValues(horizontal = 16.dp, vertical = 9.dp),
        ) {
            Text(stringResource(R.string.action_connect))
        }
    }
}
