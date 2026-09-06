<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardFooter, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import * as Dialog from '@stump/ui/components/ui/dialog';
	import { Input } from '@stump/ui/components/ui/input';
	import * as Select from '@stump/ui/components/ui/select';
	import { request } from '@stump/ui/graphql/client';
	import {
		ConsoleBooksDocument,
		ConsoleKindleDeliveriesDocument,
		ConsoleLibraryOptionsDocument,
		ConsoleSendToKindleDocument,
		RenameDeviceDocument,
		RevokeDeviceDocument,
		RotateDeviceCredentialDocument,
		SetDeviceKindleEmailDocument,
		SetDeviceLibraryScopeDocument,
		SetDeviceTransformProfileDocument
	} from '$lib/graphql/generated/graphql';
	import {
		DEVICE_KIND_LABELS,
		INHERIT_LIBRARIES,
		NO_PRESET,
		TRANSFORMABLE_KINDS,
		TRANSFORM_PRESETS,
		libraryScopeSummary,
		nextLibraryScope,
		presetOf,
		type Device,
		type IssuedCredential
	} from '$lib/devices';
	import { absoluteTime, bytesLabel, relativeTime, summarizeSync } from '$lib/format';
	import CredentialReveal from './CredentialReveal.svelte';

	let { device, now }: { device: Device; now: Date } = $props();

	/** Books offered per search in the send dialog. */
	const BOOK_PICKER_SIZE = 20;
	/** Deliveries listed on the card; the full history lives on the server. */
	const RECENT_DELIVERIES = 3;

	const queryClient = useQueryClient();
	const revoked = $derived(device.revokedAt !== null && device.revokedAt !== undefined);
	const preset = $derived(presetOf(device.transformProfile));
	const presetLabel = $derived(
		TRANSFORM_PRESETS.find((candidate) => candidate.name === preset)?.label ??
			(preset === NO_PRESET ? 'Server default' : `Custom (${preset})`)
	);
	const syncSummary = $derived(summarizeSync(device.lastSyncSummary));

	// Shares the `libraryOptions` cache entry with the entity screens, so the
	// whole card list resolves the picker's names in one request.
	const librariesQuery = createQuery(() => ({
		queryKey: ['libraryOptions'],
		queryFn: () => request(ConsoleLibraryOptionsDocument, {}),
		enabled: browser
	}));
	const libraries = $derived(librariesQuery.data?.libraries.nodes ?? []);
	// `libraryScope` is null when the device inherits its user's visibility;
	// the selector shows that as the `INHERIT_LIBRARIES` entry being picked.
	const scopeValue = $derived(
		device.libraryScope ? [...device.libraryScope] : [INHERIT_LIBRARIES]
	);
	const scopeSummary = $derived(libraryScopeSummary(device.libraryScope, libraries.length));

	let renaming = $state(false);
	let draftName = $state('');
	let rotated = $state<IssuedCredential | null>(null);
	let rotatedOpen = $state(false);
	// The address field is uncontrolled until it is edited: `null` means
	// untouched, so it always shows the stored value and a background refetch
	// can never overwrite what the operator is typing.
	let kindleEmailEdit = $state<string | null>(null);
	const storedKindleEmail = $derived(device.kindleEmail ?? '');
	const draftKindleEmail = $derived(kindleEmailEdit ?? storedKindleEmail);
	// A device is a "send to Kindle" target only once it carries an address; a
	// revoked one keeps its history but can no longer be sent to.
	const canSendToKindle = $derived(!revoked && !!device.kindleEmail);

	let sendOpen = $state(false);
	let bookSearch = $state('');
	let bookFilterText = $state('');
	let pickedBookId = $state<string | null>(null);

	// The picker queries 300ms after the last keystroke, so typing a title
	// costs one request instead of one per character.
	$effect(() => {
		const term = bookSearch.trim();
		const timer = setTimeout(() => (bookFilterText = term), 300);
		return () => clearTimeout(timer);
	});

	// Shares the book list with every other card: same search, same request.
	const booksQuery = createQuery(() => ({
		queryKey: ['kindleBookPicker', bookFilterText],
		queryFn: () =>
			request(ConsoleBooksDocument, {
				filter: bookFilterText ? { name: { contains: bookFilterText } } : {},
				orderBy: [{ media: { field: 'NAME', direction: 'ASC' } }],
				pagination: { offset: { page: 1, pageSize: BOOK_PICKER_SIZE } }
			}),
		enabled: browser && sendOpen
	}));
	const books = $derived(booksQuery.data?.media.nodes ?? []);

	const deliveriesQuery = createQuery(() => ({
		queryKey: ['kindleDeliveries', device.id],
		queryFn: () =>
			request(ConsoleKindleDeliveriesDocument, {
				deviceId: device.id,
				limit: RECENT_DELIVERIES
			}),
		enabled: browser && canSendToKindle
	}));
	const deliveries = $derived(deliveriesQuery.data?.kindleDeliveries ?? []);

	function invalidate(): void {
		void queryClient.invalidateQueries({ queryKey: ['devices'] });
		// A saved or cleared address changes which books can be sent, so the
		// library rows' target list has to be refetched too.
		void queryClient.invalidateQueries({ queryKey: ['kindleTargets'] });
		// A send, a cleared address, or a revoke all change what this device's
		// delivery history says, so the card's recent list refetches too.
		void queryClient.invalidateQueries({ queryKey: ['kindleDeliveries'] });
	}

	function failure(fallback: string): (error: unknown) => void {
		return (error) => toast.error(error instanceof Error ? error.message : fallback);
	}

	const rename = createMutation(() => ({
		mutationFn: (name: string) => request(RenameDeviceDocument, { id: device.id, name }),
		onSuccess: () => {
			renaming = false;
			invalidate();
		},
		onError: failure('Unable to rename the device.')
	}));
	const rotate = createMutation(() => ({
		mutationFn: () => request(RotateDeviceCredentialDocument, { id: device.id }),
		onSuccess: (result) => {
			rotated = result.rotateDeviceCredential;
			rotatedOpen = true;
			invalidate();
		},
		onError: failure('Unable to rotate the credential.')
	}));
	const revoke = createMutation(() => ({
		mutationFn: () => request(RevokeDeviceDocument, { id: device.id }),
		onSuccess: () => {
			toast.success(`${device.name} can no longer sign in.`);
			invalidate();
		},
		onError: failure('Unable to revoke the device.')
	}));
	const setPreset = createMutation(() => ({
		mutationFn: (name: string) =>
			request(SetDeviceTransformProfileDocument, {
				id: device.id,
				profile: name === NO_PRESET ? null : { preset: name }
			}),
		onSuccess: () => {
			toast.success(`Transform preset saved for ${device.name}.`);
			invalidate();
		},
		onError: failure('Unable to save the transform preset.')
	}));
	const setScope = createMutation(() => ({
		mutationFn: (libraryIds: string[] | null) =>
			request(SetDeviceLibraryScopeDocument, { id: device.id, libraryIds }),
		onSuccess: () => {
			toast.success(`Visible libraries saved for ${device.name}.`);
			invalidate();
		},
		onError: failure('Unable to save the visible libraries.')
	}));
	const setKindleEmail = createMutation(() => ({
		mutationFn: (email: string | null) =>
			request(SetDeviceKindleEmailDocument, { id: device.id, email }),
		onSuccess: (result) => {
			// Back to showing the stored value.
			kindleEmailEdit = null;
			toast.success(
				result.setDeviceKindleEmail.kindleEmail
					? `${device.name} will receive books at ${result.setDeviceKindleEmail.kindleEmail}.`
					: `${device.name} can no longer be sent to.`
			);
			invalidate();
		},
		onError: failure('Unable to save the Kindle address.')
	}));
	const sendBook = createMutation(() => ({
		mutationFn: (mediaId: string) =>
			request(ConsoleSendToKindleDocument, { mediaId, deviceId: device.id }),
		onSuccess: (result) => {
			const delivery = result.sendToKindle;
			toast.success(
				`Sent ${delivery.format.toUpperCase()} (${bytesLabel(delivery.bytes)}) to ${delivery.recipient}.`,
				// The note says why the book went unconverted — usually that the
				// server has no boko and Amazon converts the EPUB itself.
				{ description: delivery.note ?? undefined }
			);
			sendOpen = false;
			invalidate();
		},
		onError: failure('Unable to send the book.')
	}));

	function openSend(): void {
		pickedBookId = null;
		sendOpen = true;
	}

	function submitKindleEmail(event: SubmitEvent): void {
		event.preventDefault();
		const email = draftKindleEmail.trim();
		// An empty field clears the address rather than storing an empty one.
		setKindleEmail.mutate(email === '' ? null : email);
	}

	function applyScope(values: string[]): void {
		const next = nextLibraryScope(values, device.libraryScope);
		if (next !== undefined) setScope.mutate(next);
	}

	function startRename(): void {
		draftName = device.name;
		renaming = true;
	}

	function submitRename(event: SubmitEvent): void {
		event.preventDefault();
		const name = draftName.trim();
		if (!name || name === device.name) {
			renaming = false;
			return;
		}
		rename.mutate(name);
	}

	function confirmRevoke(): void {
		if (window.confirm(`Revoke ${device.name}? Its credential stops working immediately.`)) {
			revoke.mutate();
		}
	}

	function confirmRotate(): void {
		if (
			window.confirm(
				`Rotate the credential for ${device.name}? The current secret stops working and a new one is shown once.`
			)
		) {
			rotate.mutate();
		}
	}
</script>

<Card class={revoked ? 'opacity-70' : ''} data-device-id={device.id}>
	<CardHeader>
		<div class="flex flex-wrap items-start gap-2">
			<div class="min-w-0 flex-1">
				{#if renaming}
					<form class="flex items-center gap-2" onsubmit={submitRename}>
						<Input
							bind:value={draftName}
							maxlength={100}
							aria-label="Device name"
							autofocus
							class="h-8"
						/>
						<Button type="submit" size="sm" disabled={rename.isPending}>Save</Button>
						<Button type="button" size="sm" variant="ghost" onclick={() => (renaming = false)}>
							Cancel
						</Button>
					</form>
				{:else}
					<CardTitle class="truncate text-base">{device.name}</CardTitle>
				{/if}
			</div>
			<Badge variant="secondary">{DEVICE_KIND_LABELS[device.kind]}</Badge>
			{#if revoked}
				<Badge variant="destructive">Revoked</Badge>
			{:else if device.credential}
				<Badge variant="outline">{device.credential.protocol}</Badge>
			{/if}
		</div>
	</CardHeader>
	<CardContent class="grid gap-x-6 gap-y-1 text-sm sm:grid-cols-2">
		<div class="flex justify-between gap-2 sm:block">
			<span class="text-muted-foreground">Last seen</span>
			<span class="sm:block" title={absoluteTime(device.lastSeenAt)}>
				{relativeTime(device.lastSeenAt, now)}
			</span>
		</div>
		<div class="flex justify-between gap-2 sm:block">
			<span class="text-muted-foreground">Last sync</span>
			<span class="sm:block" title={absoluteTime(device.lastSyncAt)}>
				{relativeTime(device.lastSyncAt, now)}
			</span>
		</div>
		<div class="flex justify-between gap-2 sm:block">
			<span class="text-muted-foreground">Credential</span>
			<span class="font-mono text-xs sm:block">
				{device.credential ? device.credential.secretHint : 'none'}
			</span>
		</div>
		<div class="flex justify-between gap-2 sm:block">
			<span class="text-muted-foreground">Registered</span>
			<span class="sm:block" title={absoluteTime(device.createdAt)}>
				{relativeTime(device.createdAt, now)}
			</span>
		</div>
		{#if syncSummary}
			<p class="text-xs text-muted-foreground sm:col-span-2">{syncSummary}</p>
		{/if}
		{#if TRANSFORMABLE_KINDS[device.kind] && !revoked}
			<div class="mt-2 flex flex-col gap-1 sm:col-span-2">
				<span class="text-muted-foreground">Comic transform preset</span>
				<Select.Root
					type="single"
					value={preset}
					onValueChange={(value) => setPreset.mutate(value)}
					disabled={setPreset.isPending}
				>
					<Select.Trigger class="w-full sm:max-w-sm" aria-label="Comic transform preset">
						{presetLabel}
					</Select.Trigger>
					<Select.Content>
						<Select.Item value={NO_PRESET} label="Server default" />
						<Select.Group>
							<Select.Label>Presets</Select.Label>
							{#each TRANSFORM_PRESETS as candidate (candidate.name)}
								<Select.Item value={candidate.name} label={candidate.label} />
							{/each}
						</Select.Group>
					</Select.Content>
				</Select.Root>
			</div>
		{/if}
		{#if !revoked}
			<div class="mt-2 flex flex-col gap-1 sm:col-span-2">
				<span class="text-muted-foreground">Libraries</span>
				<Select.Root
					type="multiple"
					value={scopeValue}
					onValueChange={applyScope}
					disabled={setScope.isPending || librariesQuery.isPending}
				>
					<Select.Trigger class="w-full sm:max-w-sm" aria-label="Visible libraries">
						{scopeSummary}
					</Select.Trigger>
					<Select.Content>
						<Select.Item value={INHERIT_LIBRARIES} label="All (inherit)" />
						<Select.Group>
							<Select.Label>Restrict to</Select.Label>
							{#each libraries as library (library.id)}
								<Select.Item
									value={library.id}
									label={library.emoji ? `${library.emoji} ${library.name}` : library.name}
								/>
							{/each}
						</Select.Group>
					</Select.Content>
				</Select.Root>
			</div>
		{/if}
		{#if !revoked}
			<div class="mt-2 flex flex-col gap-1 sm:col-span-2">
				<span class="text-muted-foreground">Kindle address</span>
				<form class="flex items-center gap-2" onsubmit={submitKindleEmail}>
					<Input
						value={draftKindleEmail}
						oninput={(event) => (kindleEmailEdit = event.currentTarget.value)}
						type="email"
						maxlength={254}
						placeholder="name@kindle.com"
						aria-label="Kindle address"
						class="h-8 sm:max-w-sm"
					/>
					<Button
						type="submit"
						size="sm"
						variant="outline"
						disabled={setKindleEmail.isPending || draftKindleEmail.trim() === storedKindleEmail}
					>
						Save
					</Button>
				</form>
				<span class="text-xs text-muted-foreground">
					Books can be mailed to this device with Send to Kindle. Your emailer's sender address
					must be on Amazon's approved list; an empty field clears the address.
				</span>
			</div>
		{/if}
		{#if canSendToKindle && deliveries.length}
			<div class="mt-2 flex flex-col gap-1 sm:col-span-2">
				<span class="text-muted-foreground">Recent deliveries</span>
				<ul class="flex flex-col gap-1 text-xs">
					{#each deliveries as delivery (delivery.id)}
						<li class="flex flex-col">
							<span class={delivery.error ? 'text-muted-foreground' : ''}>
								{delivery.format.toUpperCase()} · {bytesLabel(delivery.bytes)} ·
								<span title={absoluteTime(delivery.sentAt)}>
									{relativeTime(delivery.sentAt, now)}
								</span>
							</span>
							{#if delivery.error}
								<span class="text-destructive">{delivery.error}</span>
							{/if}
						</li>
					{/each}
				</ul>
			</div>
		{/if}
	</CardContent>
	{#if !revoked}
		<CardFooter class="flex flex-wrap gap-2">
			<Button size="sm" variant="outline" onclick={startRename} disabled={renaming}>Rename</Button>
			{#if device.credential}
				<Button size="sm" variant="outline" onclick={confirmRotate} disabled={rotate.isPending}>
					{rotate.isPending ? 'Rotating…' : 'Rotate credential'}
				</Button>
			{/if}
			{#if canSendToKindle}
				<Button size="sm" variant="outline" onclick={openSend}>Send to Kindle</Button>
			{/if}
			<Button
				size="sm"
				variant="destructive"
				class="ml-auto"
				onclick={confirmRevoke}
				disabled={revoke.isPending}
			>
				Revoke
			</Button>
		</CardFooter>
	{/if}
</Card>

<Dialog.Root bind:open={rotatedOpen}>
	<Dialog.Content class="flex max-h-[85vh] flex-col overflow-hidden sm:max-w-2xl">
		<Dialog.Header>
			<Dialog.Title>New credential for {device.name}</Dialog.Title>
			<Dialog.Description>The previous secret no longer works.</Dialog.Description>
		</Dialog.Header>
		{#if rotated}
			<div class="min-h-0 flex-1 overflow-y-auto pr-1">
				<CredentialReveal issued={rotated} />
			</div>
		{/if}
		<Dialog.Footer>
			<Button onclick={() => (rotatedOpen = false)}>Done</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<Dialog.Root bind:open={sendOpen}>
	<Dialog.Content class="flex max-h-[85vh] flex-col overflow-hidden sm:max-w-2xl">
		<Dialog.Header>
			<Dialog.Title>Send a book to {device.name}</Dialog.Title>
			<Dialog.Description>
				Amazon delivers it to {storedKindleEmail}. Pick one book; the server converts it when it
				can.
			</Dialog.Description>
		</Dialog.Header>
		<Input
			bind:value={bookSearch}
			placeholder="Search books by name"
			aria-label="Search books"
			class="h-8"
		/>
		<div class="min-h-0 flex-1 overflow-y-auto pr-1">
			{#if booksQuery.isPending}
				<p class="text-sm text-muted-foreground">Loading books…</p>
			{:else if booksQuery.isError}
				<p class="text-sm text-destructive">
					{booksQuery.error instanceof Error
						? booksQuery.error.message
						: 'Unable to load the books.'}
				</p>
			{:else if !books.length}
				<p class="text-sm text-muted-foreground">
					{bookFilterText ? 'No book matches that search.' : 'This account has no books yet.'}
				</p>
			{:else}
				<ul class="flex flex-col gap-1">
					{#each books as book (book.id)}
						<li>
							<button
								type="button"
								class="flex w-full flex-col rounded-md px-2 py-1 text-left hover:bg-muted"
								class:bg-muted={pickedBookId === book.id}
								aria-pressed={pickedBookId === book.id}
								onclick={() => (pickedBookId = book.id)}
							>
								<span class="truncate text-sm font-medium">{book.resolvedName}</span>
								<span class="truncate text-xs text-muted-foreground">
									{book.series.resolvedName} · {book.extension.toUpperCase()} ·
									{bytesLabel(book.size)}
								</span>
							</button>
						</li>
					{/each}
				</ul>
			{/if}
		</div>
		<Dialog.Footer>
			<Button variant="ghost" onclick={() => (sendOpen = false)}>Cancel</Button>
			<Button
				onclick={() => pickedBookId && sendBook.mutate(pickedBookId)}
				disabled={!pickedBookId || sendBook.isPending}
			>
				{sendBook.isPending ? 'Sending…' : 'Send'}
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
