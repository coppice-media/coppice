<script lang="ts">
	import BookOpenIcon from '@lucide/svelte/icons/book-open';
	import Globe2Icon from '@lucide/svelte/icons/globe-2';
	import LinkIcon from '@lucide/svelte/icons/link';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Cover } from '@stump/ui/components/ui/cover';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import type { RequestFormat } from '$lib/graphql/generated/graphql';
	import {
		formatIncludesAudio,
		safeCoverUrl,
		safeText,
		type ExternalWorkReference,
		type NarratorLookup
	} from '$lib/requests';
	import RequestFormatControl from './RequestFormatControl.svelte';

	type Destination = { id: string; name: string };
	type Mode = 'internal' | 'external';

	type CreateInput = {
		mediaId?: string;
		workId?: string;
		external?: ExternalWorkReference;
		title?: string;
		authors?: string;
		coverUrl?: string;
		format: RequestFormat;
		preferredNarrator?: string | null;
		destinationShelfId?: string;
		destinationDeviceId?: string;
	};

	let {
		initialMode = 'external',
		initialMediaId = '',
		initialWorkId = '',
		initialProvider = '',
		initialRemoteId = '',
		initialExternalKey = '',
		initialTitle = '',
		initialAuthors = '',
		initialCoverUrl = '',
		initialFormat = 'ANY',
		initialNarrator = '',
		initialDestinationShelfId = '',
		initialDestinationDeviceId = '',
		recommendationId = '',
		targetId = '',
		devices = [],
		shelves = [],
		busy = false,
		error = null,
		oncreate
	}: {
		initialMode?: Mode;
		initialMediaId?: string;
		initialWorkId?: string;
		initialProvider?: string;
		initialRemoteId?: string;
		initialExternalKey?: string;
		initialTitle?: string;
		initialAuthors?: string;
		initialCoverUrl?: string;
		initialFormat?: RequestFormat;
		initialNarrator?: string;
		initialDestinationShelfId?: string;
		initialDestinationDeviceId?: string;
		recommendationId?: string;
		targetId?: string;
		devices?: readonly Destination[];
		shelves?: readonly Destination[];
		busy?: boolean;
		error?: string | null;
		oncreate?: (input: CreateInput) => void;
	} = $props();

	// These route handoff values seed the editable form once. Keeping the seed
	// separate from the draft state means parent prop updates do not overwrite
	// edits made while the form is open.
	let mode = $state<Mode>('external');
	let mediaId = $state('');
	let workId = $state('');
	let provider = $state('');
	let remoteId = $state('');
	let externalKey = $state('');
	let title = $state('');
	let authors = $state('');
	let coverUrl = $state('');
	let format = $state<RequestFormat>('ANY');
	let narrator = $state<string | null>(null);
	let destinationShelfId = $state('');
	let destinationDeviceId = $state('');

	function initializeFormState(): void {
		mode = initialMode;
		mediaId = initialMediaId;
		workId = initialWorkId;
		provider = initialProvider;
		remoteId = initialRemoteId;
		externalKey = initialExternalKey;
		title = initialTitle;
		authors = initialAuthors;
		coverUrl = initialCoverUrl;
		format = initialFormat;
		narrator = initialNarrator || null;
		destinationShelfId = initialDestinationShelfId;
		destinationDeviceId = initialDestinationDeviceId;
	}

	initializeFormState();

	const safeCover = $derived(safeCoverUrl(coverUrl));
	// The narrator lookup needs the provider reference the external mode collects.
	const narratorLookup = $derived<NarratorLookup | null>(
		mode === 'external' && provider.trim() && remoteId.trim() && title.trim()
			? { provider: provider.trim(), remoteId: remoteId.trim(), title: title.trim(), authors: authors.trim() || null }
			: null
	);
	const valid = $derived(
		mode === 'internal'
			? Boolean(mediaId.trim() || workId.trim())
			: Boolean(provider.trim() && remoteId.trim() && title.trim())
	);

	function submit(event: SubmitEvent): void {
		event.preventDefault();
		if (!valid || busy) return;
		const input: CreateInput = {
			title: safeText(title, 240) || undefined,
			authors: safeText(authors, 240) || undefined,
			coverUrl: safeCover,
			format,
			preferredNarrator: formatIncludesAudio(format) ? narrator : null,
			destinationShelfId: destinationShelfId || undefined,
			destinationDeviceId: destinationDeviceId || undefined,
		};
		if (mode === 'internal') {
			input.mediaId = mediaId.trim() || undefined;
			input.workId = workId.trim() || undefined;
		} else {
			input.external = {
				sourceProvider: safeText(provider, 80),
				remoteId: safeText(remoteId, 160),
				externalKey: safeText(externalKey, 240) || undefined,
				title: safeText(title, 240),
				authors: safeText(authors, 240) || undefined,
				coverUrl: safeCover
			};
		}
		oncreate?.(input);
	}
</script>

<Card>
	<CardHeader>
		<CardTitle class="text-base">Request a book</CardTitle>
		<CardDescription>
			Choose something already known to this server, or hand off metadata from a trusted recommendation.
		</CardDescription>
		{#if recommendationId || targetId}
			<Alert class="mt-3">
				<AlertTitle>Recommendation handoff</AlertTitle>
				<AlertDescription>
					Review the provider and title below before submitting. The opaque recommendation reference stays with the social flow; this form accepts metadata and destination choices only.
				</AlertDescription>
			</Alert>
		{/if}
	</CardHeader>
	<CardContent>
		{#if error}
			<Alert variant="destructive" class="mb-4">
				<AlertTitle>Request could not be created</AlertTitle>
				<AlertDescription>{error}</AlertDescription>
			</Alert>
		{/if}

		<form class="grid gap-5" onsubmit={submit}>
			<div class="grid gap-2 sm:grid-cols-2" role="radiogroup" aria-label="Request source">
				<button
					type="button"
					role="radio"
					aria-checked={mode === 'internal'}
					onclick={() => (mode = 'internal')}
					class="flex items-start gap-3 rounded-lg border p-3 text-left transition-colors {mode === 'internal' ? 'border-primary bg-primary/5' : 'bg-background/40'}"
				>
					<BookOpenIcon class="mt-0.5 size-4 shrink-0" aria-hidden="true" />
					<span><strong class="block text-sm font-medium">From this library</strong><span class="text-xs text-muted-foreground">Use an existing media or work id.</span></span>
				</button>
				<button
					type="button"
					role="radio"
					aria-checked={mode === 'external'}
					onclick={() => (mode = 'external')}
					class="flex items-start gap-3 rounded-lg border p-3 text-left transition-colors {mode === 'external' ? 'border-primary bg-primary/5' : 'bg-background/40'}"
				>
					<Globe2Icon class="mt-0.5 size-4 shrink-0" aria-hidden="true" />
					<span><strong class="block text-sm font-medium">External work</strong><span class="text-xs text-muted-foreground">Metadata from a trusted catalog or recommendation.</span></span>
				</button>
			</div>

			{#if mode === 'internal'}
				<div class="grid gap-4 rounded-lg border p-4 sm:grid-cols-2">
					<div class="flex flex-col gap-2">
						<Label for="request-media-id">Media id <span class="font-normal text-muted-foreground">(optional)</span></Label>
						<Input id="request-media-id" bind:value={mediaId} placeholder="Existing book/media id" />
					</div>
					<div class="flex flex-col gap-2">
						<Label for="request-work-id">Work id <span class="font-normal text-muted-foreground">(optional)</span></Label>
						<Input id="request-work-id" bind:value={workId} placeholder="Existing work id" />
					</div>
					<div class="flex flex-col gap-2 sm:col-span-2">
						<Label for="request-internal-title">Title <span class="font-normal text-muted-foreground">(optional)</span></Label>
						<Input id="request-internal-title" bind:value={title} placeholder="Used when the id has no display title" />
					</div>
				</div>
			{:else}
				<div class="grid gap-4 rounded-lg border p-4 sm:grid-cols-2">
					<div class="flex flex-col gap-2">
						<Label for="request-provider">Source provider</Label>
						<Input id="request-provider" bind:value={provider} required placeholder="hardcover" autocomplete="off" />
					</div>
					<div class="flex flex-col gap-2">
						<Label for="request-remote-id">Remote id</Label>
						<Input id="request-remote-id" bind:value={remoteId} required placeholder="Provider's work id" autocomplete="off" />
					</div>
					<div class="flex flex-col gap-2 sm:col-span-2">
						<Label for="request-external-key">External key <span class="font-normal text-muted-foreground">(optional)</span></Label>
						<Input id="request-external-key" bind:value={externalKey} placeholder="ISBN, edition key, or provider slug" autocomplete="off" />
					</div>
				</div>
			{/if}

			<div class="grid gap-4 rounded-lg border p-4 sm:grid-cols-2">
				<div class="flex flex-col gap-2 sm:col-span-2">
					<Label for="request-title">Title</Label>
					<Input id="request-title" bind:value={title} required={mode === 'external'} placeholder="Book title" />
				</div>
				<div class="flex flex-col gap-2">
					<Label for="request-authors">Authors <span class="font-normal text-muted-foreground">(optional)</span></Label>
					<Input id="request-authors" bind:value={authors} placeholder="Author names" />
				</div>
				<div class="flex flex-col gap-2">
					<Label for="request-cover">Cover URL <span class="font-normal text-muted-foreground">(optional)</span></Label>
					<Input id="request-cover" bind:value={coverUrl} type="url" placeholder="https://…" />
				</div>
				<div class="flex flex-col gap-2" role="group" aria-labelledby="request-format-label">
					<span id="request-format-label" class="text-sm leading-none font-medium">Format</span>
					<RequestFormatControl bind:value={format} bind:narrator lookup={narratorLookup} label="Request format" />
				</div>
				{#if safeCover}
					<div class="flex items-center gap-3 sm:col-span-2">
						<Cover src={safeCover} class="size-14 rounded border" />
						<span class="text-xs text-muted-foreground">Cover preview saved with the request metadata.</span>
					</div>
				{/if}
			</div>

			<div class="grid gap-4 rounded-lg border p-4 sm:grid-cols-2">
				<div class="flex flex-col gap-2">
					<Label for="request-shelf">Destination shelf <span class="font-normal text-muted-foreground">(optional)</span></Label>
					<select id="request-shelf" bind:value={destinationShelfId} class="h-9 rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50">
						<option value="">No shelf</option>
						{#each shelves as shelf (shelf.id)}
							<option value={shelf.id}>{shelf.name}</option>
						{/each}
					</select>
				</div>
				<div class="flex flex-col gap-2">
					<Label for="request-device">Destination device <span class="font-normal text-muted-foreground">(optional)</span></Label>
					<select id="request-device" bind:value={destinationDeviceId} class="h-9 rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50">
						<option value="">No device</option>
						{#each devices as device (device.id)}
							<option value={device.id}>{device.name}</option>
						{/each}
					</select>
				</div>
			</div>

			<div class="flex flex-wrap items-center justify-between gap-3 border-t pt-4">
				<p class="flex items-center gap-1.5 text-xs text-muted-foreground"><LinkIcon class="size-3.5" aria-hidden="true" /> Coppice records metadata, destinations, and the approval decision.</p>
				<Button type="submit" disabled={!valid || busy}>{busy ? 'Submitting…' : 'Submit request'}</Button>
			</div>
		</form>
	</CardContent>
</Card>
