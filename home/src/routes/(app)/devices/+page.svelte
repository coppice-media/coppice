<script lang="ts">
	import { browser } from '$app/environment';
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Button } from '@stump/ui/components/ui/button';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { request, subscribe } from '@stump/ui/graphql/client';
	import { DeviceSeenDocument, DevicesDocument } from '$lib/graphql/generated/graphql';
	import AddClientDialog from '$lib/components/AddClientDialog.svelte';
	import DeviceCard from '$lib/components/DeviceCard.svelte';
	import PendingPairings from '$lib/components/PendingPairings.svelte';

	const queryClient = useQueryClient();
	const devicesQuery = createQuery(() => ({
		queryKey: ['devices'],
		queryFn: () => request(DevicesDocument, {}),
		enabled: browser
	}));

	// Active devices first, then revoked; newest registration on top within each group.
	const devices = $derived(
		[...(devicesQuery.data?.devices ?? [])].sort((a, b) => {
			const revokedA = a.revokedAt ? 1 : 0;
			const revokedB = b.revokedAt ? 1 : 0;
			if (revokedA !== revokedB) return revokedA - revokedB;
			return b.createdAt.localeCompare(a.createdAt);
		})
	);

	let addOpen = $state(false);
	let liveError = $state<string | null>(null);
	// Ticks once a minute so "last seen 3m ago" stays honest without refetching.
	let now = $state(new Date());

	$effect(() => {
		const timer = setInterval(() => (now = new Date()), 60_000);
		return () => clearInterval(timer);
	});

	// A device authenticating anywhere updates its last-seen timestamp; refetch
	// the list so the card reflects it within a round-trip.
	$effect(() => {
		return subscribe(DeviceSeenDocument, {}, {
			next: (result) => {
				if (!result.data?.deviceSeen) return;
				now = new Date();
				void queryClient.invalidateQueries({ queryKey: ['devices'] });
			},
			error: () => {
				liveError = 'Live device activity is unavailable; last-seen times update on reload.';
			},
			complete: () => undefined
		});
	});
</script>

<svelte:head>
	<title>Devices · Stump</title>
</svelte:head>

<div class="flex flex-col gap-8">
	<div class="flex flex-wrap items-center gap-3">
		<div class="mr-auto">
			<h1 class="text-2xl font-semibold tracking-tight">Devices</h1>
			<p class="text-sm text-muted-foreground">
				Readers and integrations signed in with a credential of their own.
			</p>
		</div>
		<Button onclick={() => (addOpen = true)}>
			<PlusIcon data-icon="inline-start" />
			Add client
		</Button>
	</div>

	{#if liveError}
		<Alert>
			<AlertTitle>Live updates paused</AlertTitle>
			<AlertDescription>{liveError}</AlertDescription>
		</Alert>
	{/if}

	<PendingPairings {now} />

	{#if devicesQuery.isPending}
		<div class="grid gap-4 md:grid-cols-2">
			<Skeleton class="h-48 rounded-xl" />
			<Skeleton class="h-48 rounded-xl" />
		</div>
	{:else if devicesQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load devices</AlertTitle>
			<AlertDescription>
				{devicesQuery.error instanceof Error ? devicesQuery.error.message : 'Request failed.'}
			</AlertDescription>
		</Alert>
	{:else if devices.length === 0}
		<Empty class="rounded-xl border border-dashed bg-card">
			<EmptyHeader>
				<EmptyTitle>No devices yet</EmptyTitle>
				<EmptyDescription>
					Add a client to mint a credential for your Kobo, KOReader, Liseur, Mihon, Komelia, or
					any OPDS reader.
				</EmptyDescription>
			</EmptyHeader>
		</Empty>
	{:else}
		<div class="grid gap-4 md:grid-cols-2">
			{#each devices as device (device.id)}
				<DeviceCard {device} {now} />
			{/each}
		</div>
	{/if}
</div>

<AddClientDialog bind:open={addOpen} />
