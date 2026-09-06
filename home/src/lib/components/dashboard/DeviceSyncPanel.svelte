<script lang="ts">
	import { resolve } from '$app/paths';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { DEVICE_KIND_LABELS, type Device } from '$lib/devices';
	import { absoluteTime, relativeTime, summarizeSync } from '$lib/format';
	import DashboardSection from './DashboardSection.svelte';

	// Same rows the Devices screen renders on its cards, condensed: the
	// dashboard only answers "did this device sync, and when".
	let {
		devices,
		pending = false,
		error = null,
		now = new Date()
	}: {
		devices: readonly Device[];
		pending?: boolean;
		error?: unknown;
		now?: Date;
	} = $props();

	const active = $derived(devices.filter((device) => !device.revokedAt));
	const synced = $derived(active.filter((device) => device.lastSyncAt).length);
</script>

<DashboardSection
	title="Device sync"
	description={active.length
		? `${synced} of ${active.length} ${active.length === 1 ? 'device has' : 'devices have'} synced at least once.`
		: undefined}
	{pending}
	{error}
	errorTitle="Unable to load devices"
	emptyTitle="No devices registered"
	emptyDescription="Register a Kobo, KOReader, Liseur, or OPDS client to sync reading progress."
	empty={active.length === 0}
>
	{#snippet action()}
		<Button href={resolve('/devices')} size="sm" variant="outline">Manage</Button>
	{/snippet}
	<ul class="flex flex-col divide-y">
		{#each active as device (device.id)}
			{@const summary = summarizeSync(device.lastSyncSummary)}
			<li class="flex flex-wrap items-baseline gap-x-2 gap-y-1 py-2 first:pt-0 last:pb-0">
				<span class="min-w-0 truncate text-sm font-medium">{device.name}</span>
				<Badge variant="secondary">{DEVICE_KIND_LABELS[device.kind]}</Badge>
				{#if device.credential}
					<Badge variant="outline">{device.credential.protocol}</Badge>
				{/if}
				<span class="ml-auto text-xs text-muted-foreground" title={absoluteTime(device.lastSyncAt)}>
					{device.lastSyncAt ? `synced ${relativeTime(device.lastSyncAt, now)}` : 'never synced'}
				</span>
				<span
					class="w-full text-xs text-muted-foreground"
					title={absoluteTime(device.lastSeenAt)}
				>
					Last seen {relativeTime(device.lastSeenAt, now)}{summary ? ` · ${summary}` : ''}
				</span>
			</li>
		{/each}
	</ul>
</DashboardSection>
