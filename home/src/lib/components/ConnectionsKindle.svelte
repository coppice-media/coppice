<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import CheckIcon from '@lucide/svelte/icons/check';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';
	import SendIcon from '@lucide/svelte/icons/send';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { Switch } from '@stump/ui/components/ui/switch';
	import { request } from '@stump/ui/graphql/client';
	import {
		ConnectionDeleteKindleDestinationDocument,
		ConnectionKindleDestinationDeliveriesDocument,
		ConnectionKindleDestinationsDocument,
		ConnectionSendToKindleDestinationDocument,
		ConnectionUpsertKindleDestinationDocument
	} from '$lib/graphql/generated/graphql';
	import { absoluteTime, relativeTime } from '$lib/format';
	const queryClient = useQueryClient();
	const destinationsQuery = createQuery(() => ({
		queryKey: ['kindle-destinations'],
		queryFn: () => request(ConnectionKindleDestinationsDocument, {}),
		enabled: browser
	}));
	const destinations = $derived(destinationsQuery.data?.kindleDestinations ?? []);
	const deliveriesQuery = createQuery(() => ({
		queryKey: ['kindle-destination-deliveries'],
		queryFn: () =>
			request(ConnectionKindleDestinationDeliveriesDocument, { mediaId: null, limit: 20 }),
		enabled: browser
	}));
	const deliveries = $derived(deliveriesQuery.data?.kindleDestinationDeliveries ?? []);

	let editingId = $state<string | null>(null);
	let name = $state('');
	let email = $state('');
	let makeDefault = $state(false);
	let saving = $state(false);
	let deletingId = $state<string | null>(null);
	let sendMediaId = $state('');
	let sendDestinationId = $state('');
	let sending = $state(false);
	let sendResult = $state<{
		recipient: string;
		format: string;
		bytes: number;
		error: string | null;
	} | null>(null);

	const saveDestination = createMutation(() => ({
		mutationFn: () =>
			request(ConnectionUpsertKindleDestinationDocument, {
				id: editingId,
				name: name.trim(),
				email: email.trim(),
				makeDefault
			}),
		onSuccess: () => {
			resetForm();
			void queryClient.invalidateQueries({ queryKey: ['kindle-destinations'] });
			toast.success('Kindle destination saved.');
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to save Kindle destination.'),
		onSettled: () => (saving = false)
	}));

	const deleteDestination = createMutation(() => ({
		mutationFn: (id: string) => request(ConnectionDeleteKindleDestinationDocument, { id }),
		onSuccess: () => {
			void queryClient.invalidateQueries({ queryKey: ['kindle-destinations'] });
			toast.success('Kindle destination deleted. Delivery history was retained.');
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to delete Kindle destination.'),
		onSettled: () => (deletingId = null)
	}));

	const sendToKindle = createMutation(() => ({
		mutationFn: () =>
			request(ConnectionSendToKindleDestinationDocument, {
				mediaId: sendMediaId.trim(),
				destinationId: sendDestinationId
			}),
		onSuccess: (result) => {
			const delivery = result.sendToKindleDestination;
			sendResult = {
				recipient: delivery.recipient,
				format: delivery.format,
				bytes: delivery.bytes,
				error: delivery.error
			};
			void queryClient.invalidateQueries({ queryKey: ['kindle-destination-deliveries'] });
			if (delivery.error) toast.error('Kindle delivery recorded with an error.');
			else toast.success('Book sent to Kindle.');
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to send the book to Kindle.'),
		onSettled: () => (sending = false)
	}));

	function resetForm(): void {
		editingId = null;
		name = '';
		email = '';
		makeDefault = destinations.length === 0;
	}

	function editDestination(destination: (typeof destinations)[number]): void {
		editingId = destination.id;
		name = destination.name;
		email = destination.email;
		makeDefault = destination.isDefault;
	}

	function submitDestination(event: SubmitEvent): void {
		event.preventDefault();
		if (!name.trim() || !email.trim()) return;
		saving = true;
		saveDestination.mutate();
	}

	function removeDestination(id: string): void {
		if (!browser || !window.confirm('Delete this Kindle destination? Existing delivery history will remain.')) return;
		deletingId = id;
		deleteDestination.mutate(id);
	}

	function submitSend(event: SubmitEvent): void {
		event.preventDefault();
		if (!sendMediaId.trim() || !sendDestinationId) return;
		sendResult = null;
		sending = true;
		sendToKindle.mutate();
	}

	function bytesLabel(bytes: number): string {
		if (bytes < 1024) return `${bytes} B`;
		if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
		return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
	}
</script>

<div class="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(20rem,0.8fr)]">
	<Card>
		<CardHeader>
			<div class="flex flex-wrap items-start gap-2">
				<div class="mr-auto">
					<CardTitle class="text-base">Kindle destinations</CardTitle>
					<CardDescription>
						Save one or more Amazon Send to Kindle addresses for your account. A destination is not a device credential.
					</CardDescription>
				</div>
				<Badge variant="secondary">{destinations.length}</Badge>
			</div>
		</CardHeader>
		<CardContent class="flex flex-col gap-4">
			{#if destinationsQuery.isPending}
				<div class="flex flex-col gap-3" aria-label="Loading Kindle destinations">
					<Skeleton class="h-16 w-full rounded-lg" />
					<Skeleton class="h-16 w-full rounded-lg" />
				</div>
			{:else if destinationsQuery.isError}
				<Alert variant="destructive">
					<AlertTitle>Unable to load Kindle destinations</AlertTitle>
					<AlertDescription>
						{destinationsQuery.error instanceof Error ? destinationsQuery.error.message : 'Request failed.'}
					</AlertDescription>
					<Button type="button" size="sm" variant="outline" class="mt-3" onclick={() => destinationsQuery.refetch()}>
						<RotateCcwIcon data-icon="inline-start" />
						Retry
					</Button>
				</Alert>
			{:else if destinations.length === 0}
				<Empty class="rounded-lg border border-dashed">
					<EmptyHeader>
						<EmptyTitle>No Kindle destinations</EmptyTitle>
						<EmptyDescription>Add the address Amazon gave your Kindle app or device.</EmptyDescription>
					</EmptyHeader>
				</Empty>
			{:else}
				<ul class="flex flex-col gap-2" aria-label="Kindle destinations">
					{#each destinations as destination (destination.id)}
						<li class="flex flex-wrap items-center gap-3 rounded-lg border p-3">
							<div class="min-w-0 flex-1">
								<div class="flex flex-wrap items-center gap-2">
									<span class="truncate text-sm font-medium">{destination.name}</span>
									{#if destination.isDefault}<Badge><CheckIcon aria-hidden="true" />Default</Badge>{/if}
								</div>
								<code class="break-all text-xs text-muted-foreground">{destination.email}</code>
							</div>
							<div class="flex items-center gap-1">
								<Button size="icon-sm" variant="ghost" aria-label={`Edit ${destination.name}`} onclick={() => editDestination(destination)}>
									<PencilIcon aria-hidden="true" />
								</Button>
								<Button
									size="icon-sm"
									variant="ghost"
									aria-label={`Delete ${destination.name}`}
									disabled={deletingId === destination.id}
									onclick={() => removeDestination(destination.id)}
								>
									<Trash2Icon aria-hidden="true" />
								</Button>
							</div>
						</li>
					{/each}
				</ul>
			{/if}

			<form class="grid gap-3 rounded-lg border bg-muted/20 p-4" onsubmit={submitDestination}>
				<div>
					<h3 class="text-sm font-medium">{editingId ? 'Edit destination' : 'Add destination'}</h3>
					<p class="text-xs text-muted-foreground">Only the Amazon-approved recipient address belongs here.</p>
				</div>
				<div class="grid gap-3 sm:grid-cols-2">
					<div class="grid gap-2">
						<Label for="kindle-destination-name">Name</Label>
						<Input id="kindle-destination-name" bind:value={name} maxlength={80} autocomplete="off" placeholder="My Kindle" required />
					</div>
					<div class="grid gap-2">
						<Label for="kindle-destination-email">Kindle recipient address</Label>
						<Input id="kindle-destination-email" bind:value={email} type="email" autocomplete="email" placeholder="name_123@kindle.com" required />
					</div>
				</div>
				<div class="flex flex-wrap items-center justify-between gap-3">
					<Label for="kindle-destination-default" class="flex items-center gap-2 text-sm font-normal">
						<Switch id="kindle-destination-default" bind:checked={makeDefault} />
						Use as default destination
					</Label>
					<div class="flex gap-2">
						{#if editingId}<Button type="button" size="sm" variant="ghost" onclick={resetForm}>Cancel</Button>{/if}
						<Button type="submit" size="sm" disabled={saving || !name.trim() || !email.trim()}>
							{saving ? 'Saving…' : editingId ? 'Save changes' : 'Add destination'}
						</Button>
					</div>
				</div>
			</form>
		</CardContent>
	</Card>

	<Card>
		<CardHeader>
			<CardTitle class="text-base">How Send to Kindle works</CardTitle>
			<CardDescription>One server sender, many personal destinations.</CardDescription>
		</CardHeader>
		<CardContent class="flex flex-col gap-3 text-sm text-muted-foreground">
			<p>
				Coppice sends the book through the server’s one administrator-managed SMTP sender. Amazon accepts it only when that sender is in your Amazon-approved email list.
			</p>
			<p>
				Your Kindle address is stored privately on your account. It is never used as an SMTP credential and it does not make a generic sync device.
			</p>
			<p class="rounded-lg border border-dashed bg-muted/30 p-3 text-xs">
				If delivery fails, verify both sides: the administrator’s SMTP sender and the approved-sender list in Amazon’s Kindle settings.
			</p>
		</CardContent>
	</Card>
</div>

<Card>
	<CardHeader>
		<CardTitle class="text-base">Send a book</CardTitle>
		<CardDescription>Enter a visible media ID to test a destination from this account.</CardDescription>
	</CardHeader>
	<CardContent>
		<form class="grid gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(12rem,0.7fr)_auto] sm:items-end" onsubmit={submitSend}>
			<div class="grid gap-2">
				<Label for="kindle-media-id">Media ID</Label>
				<Input id="kindle-media-id" bind:value={sendMediaId} placeholder="Book media ID" autocomplete="off" required />
			</div>
			<div class="grid gap-2">
				<Label for="kindle-send-destination">Destination</Label>
				<select id="kindle-send-destination" bind:value={sendDestinationId} class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm" required>
					<option value="" disabled>Select a destination</option>
					{#each destinations as destination (destination.id)}
						<option value={destination.id}>{destination.name}</option>
					{/each}
				</select>
			</div>
			<Button type="submit" disabled={sending || !sendMediaId.trim() || !sendDestinationId}>
				<SendIcon data-icon="inline-start" />
				{sending ? 'Sending…' : 'Send to Kindle'}
			</Button>
		</form>
		{#if sendResult}
			<Alert class="mt-4" variant={sendResult.error ? 'destructive' : 'default'}>
				<AlertTitle>{sendResult.error ? 'Delivery recorded with an error' : 'Delivery accepted'}</AlertTitle>
				<AlertDescription>
					{sendResult.error ?? `${sendResult.format.toUpperCase()} attachment · ${bytesLabel(sendResult.bytes)} · ${sendResult.recipient}`}
				</AlertDescription>
			</Alert>
		{/if}
	</CardContent>
</Card>

<Card>
	<CardHeader>
		<CardTitle class="text-base">Recent destination deliveries</CardTitle>
		<CardDescription>Deleting a destination does not delete this history.</CardDescription>
	</CardHeader>
	<CardContent>
		{#if deliveriesQuery.isPending}
			<div class="flex flex-col gap-2" aria-label="Loading Kindle delivery history">
				<Skeleton class="h-10 w-full rounded-lg" />
				<Skeleton class="h-10 w-full rounded-lg" />
			</div>
		{:else if deliveriesQuery.isError}
			<Alert variant="destructive">
				<AlertTitle>Unable to load delivery history</AlertTitle>
				<AlertDescription>{deliveriesQuery.error instanceof Error ? deliveriesQuery.error.message : 'Request failed.'}</AlertDescription>
				<Button type="button" size="sm" variant="outline" class="mt-3" onclick={() => deliveriesQuery.refetch()}>Retry</Button>
			</Alert>
		{:else if deliveries.length === 0}
			<p class="text-sm text-muted-foreground">No destination deliveries yet.</p>
		{:else}
			<ul class="flex flex-col divide-y rounded-lg border text-sm" aria-label="Recent Kindle destination deliveries">
				{#each deliveries as delivery (delivery.id)}
					<li class="flex flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2.5">
						<div class="min-w-0 flex-1">
							<p class="truncate font-medium">{delivery.destinationName ?? delivery.recipient}</p>
							<p class="text-xs text-muted-foreground">{delivery.format.toUpperCase()} · {bytesLabel(delivery.bytes)} · {delivery.error ? 'Failed' : 'Accepted'}</p>
						</div>
						<time class="shrink-0 text-xs text-muted-foreground" datetime={delivery.sentAt} title={absoluteTime(delivery.sentAt)}>{relativeTime(delivery.sentAt)}</time>
					</li>
				{/each}
			</ul>
		{/if}
	</CardContent>
</Card>
