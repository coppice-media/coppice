<script lang="ts">
	/**
	 * The dashboard's answer to "which of my devices synced, and when": one
	 * row per registered device with its kind, typed telemetry, and cumulative
	 * activity. Missing observations stay missing instead of becoming guesses.
	 */
	import { resolve } from '$app/paths';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import { Button } from '@stump/ui/components/ui/button';
	import {
		DEVICE_KIND_ICONS,
		DEVICE_KIND_LABELS,
		PROTOCOL_LABELS,
		type Device
	} from '$lib/devices';
	import { absoluteTime, countLabel, relativeTime } from '$lib/format';
	import Widget from './Widget.svelte';

	type DeviceTelemetryCounters = {
		progress?: number | null;
		highlights?: number | null;
		notes?: number | null;
		bookmarks?: number | null;
		sessions?: number | null;
		items?: number | null;
	};

	type DeviceTelemetry = {
		batteryPercent?: number | null;
		charging?: boolean | null;
		batterySource?: string | null;
		batteryObservedAt?: string | null;
		syncStatus?: string | null;
		syncProtocol?: string | null;
		syncedAt?: string | null;
		counters?: DeviceTelemetryCounters | null;
	};

	/**
	 * The generated Device type intentionally lags until the home codegen pass
	 * runs after the server schema changes. Keep this adapter narrow and
	 * optional so the dashboard remains safe during that transition.
	 */
	type DashboardDevice = Device & { telemetry?: DeviceTelemetry | null };

	const COUNTER_LABELS: readonly { key: keyof DeviceTelemetryCounters; label: string }[] = [
		{ key: 'progress', label: 'Progress' },
		{ key: 'highlights', label: 'Highlights' },
		{ key: 'notes', label: 'Notes' },
		{ key: 'bookmarks', label: 'Bookmarks' },
		{ key: 'sessions', label: 'Sessions' },
		{ key: 'items', label: 'Items' }
	];

	let {
		devices,
		query,
		now = new Date(),
		class: className
	}: {
		devices: readonly DashboardDevice[];
		query: { isPending: boolean; error: unknown; refetch: () => unknown };
		now?: Date;
		class?: string;
	} = $props();

	function timestamp(value: string | null | undefined): number {
		if (!value) return 0;
		const parsed = new Date(value).getTime();
		return Number.isNaN(parsed) ? 0 : parsed;
	}

	function syncTimestamp(device: DashboardDevice): string | null {
		return device.telemetry?.syncedAt ?? device.lastSyncAt ?? null;
	}

	function humanize(value: string | null | undefined): string | null {
		if (!value?.trim()) return null;
		return value
			.trim()
			.toLowerCase()
			.replace(/[_-]+/g, ' ')
			.replace(/\b\w/g, (character) => character.toUpperCase());
	}

	function protocolLabel(value: string | null | undefined): string | null {
		if (!value) return null;
		return PROTOCOL_LABELS[value as keyof typeof PROTOCOL_LABELS] ?? humanize(value);
	}

	function syncDetails(telemetry: DeviceTelemetry | null | undefined): string | null {
		if (!telemetry) return null;
		return [protocolLabel(telemetry.syncProtocol), humanize(telemetry.syncStatus)]
			.filter((value): value is string => Boolean(value))
			.join(' · ') || null;
	}

	function observedBattery(telemetry: DeviceTelemetry | null | undefined): string | null {
		const observedAt = telemetry?.batteryObservedAt;
		if (!observedAt || timestamp(observedAt) === 0) return null;

		const parts: string[] = [];
		if (
			typeof telemetry?.batteryPercent === 'number' &&
			Number.isFinite(telemetry.batteryPercent) &&
			telemetry.batteryPercent >= 0 &&
			telemetry.batteryPercent <= 100
		) {
			parts.push(`${Math.round(telemetry.batteryPercent)}%`);
		}
		if (telemetry?.charging === true) parts.push('Charging');
		else if (telemetry?.charging === false) parts.push('Not charging');
		if (telemetry?.batterySource) {
			parts.push(humanize(telemetry.batterySource) ?? telemetry.batterySource);
		}

		return `${parts.length ? `Battery ${parts.join(' · ')}` : 'Battery observed'} · observed ${relativeTime(
			observedAt,
			now
		)}`;
	}

	function counterEntries(
		counters: DeviceTelemetryCounters | null | undefined
	): { label: string; value: string }[] {
		if (!counters) return [];
		return COUNTER_LABELS.flatMap(({ key, label }) => {
			const value = counters[key];
			return typeof value === 'number' && Number.isFinite(value) && value >= 0
				? [{ label, value: countLabel(value) }]
				: [];
		});
	}

	// Revoked devices keep their history on the Devices screen; the dashboard
	// only lists the ones that can still sync, most recently active first.
	const active = $derived(
		devices
			.filter((device) => !device.revokedAt)
			.toSorted(
				(left, right) =>
					timestamp(syncTimestamp(right)) -
					timestamp(syncTimestamp(left)) ||
					timestamp(right.lastSeenAt) - timestamp(left.lastSeenAt)
			)
	);

	const syncedCount = $derived(active.filter((device) => syncTimestamp(device)).length);
</script>

<Widget
	title="Your devices"
	description={active.length ? `${syncedCount} of ${active.length} synced at least once.` : undefined}
	href={resolve('/devices')}
	{query}
	empty={active.length === 0}
	emptyTitle="No devices yet"
	emptyDescription="Pair a Kobo, KOReader, Liseur, or any OPDS reader and its progress syncs here."
	errorTitle="Unable to load devices"
	rows={4}
	class={className}
>
	{#snippet emptyActions()}
		<Button size="sm" variant="outline" href={resolve('/devices')}>
			<PlusIcon data-icon="inline-start" aria-hidden="true" />
			Add a device
		</Button>
	{/snippet}
	<ul class="-my-1 flex flex-col divide-y">
		{#each active as device (device.id)}
			{@const Icon = DEVICE_KIND_ICONS[device.kind]}
			{@const telemetry = device.telemetry}
			{@const syncAt = syncTimestamp(device)}
			{@const details = syncDetails(telemetry)}
			{@const battery = observedBattery(telemetry)}
			{@const counters = counterEntries(telemetry?.counters)}
			<li class="flex min-w-0 items-start gap-3 py-3">
				<span
					class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground"
					title={DEVICE_KIND_LABELS[device.kind]}
				>
					<Icon class="size-4" aria-hidden="true" />
					<span class="sr-only">{DEVICE_KIND_LABELS[device.kind]}</span>
				</span>
				<div class="flex min-w-0 flex-1 flex-col gap-1">
					<div class="flex items-baseline gap-2">
						<span class="min-w-0 truncate text-sm font-medium">{device.name}</span>
					</div>
					{#if battery}
						<span class="truncate text-xs text-muted-foreground" title={absoluteTime(telemetry?.batteryObservedAt)}>
							{battery}
						</span>
					{/if}
					<div class="flex flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
						{#if details}<span>{details}</span>{/if}
						{#if syncAt}
							<span title={absoluteTime(syncAt)}>Synced {relativeTime(syncAt, now)}</span>
						{:else}
							<span>Not synced yet</span>
						{/if}
					</div>
					{#if counters.length}
						<dl class="grid grid-cols-2 gap-x-3 gap-y-1 pt-1 text-xs @sm/widget:grid-cols-3">
							{#each counters as counter (counter.label)}
								<div class="flex min-w-0 items-baseline justify-between gap-2">
									<dt class="truncate text-muted-foreground">{counter.label}</dt>
									<dd class="shrink-0 tabular-nums">{counter.value}</dd>
								</div>
							{/each}
						</dl>
					{/if}
				</div>
			</li>
		{/each}
	</ul>
</Widget>
