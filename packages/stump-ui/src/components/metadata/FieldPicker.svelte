
<script lang="ts">
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import { Textarea } from '@stump/ui/components/ui/textarea';
	import {
		buildSharedSavePlan,
		candidateOffers,
		changedFields,
		emptyDraft,
		isLocked,
		providerLabel,
		sameValue,
		validateText,
		visibleFields,
		type CandidateLike,
		type Draft,
		type SharedEditorMode,
		type SharedFieldDescriptor,
		type SharedFieldKey,
		type SharedPublicationKind,
		type SharedSavePlan
	} from '@stump/ui/editor-fields';

type MetadataSearchInput = { title?: string; author?: string; isbn?: string; limit?: number };
type MetadataCandidateApplyInput = { candidateIndex: number; selectedFields: string[] };
type MetadataFieldPickerProps = {
	mode?: SharedEditorMode;
	kind?: SharedPublicationKind;
	fields?: readonly SharedFieldDescriptor[];
	baseline: Draft;
	resetKey?: string;
	staged?: ReadonlySet<SharedFieldKey>;
	lockedFields?: readonly string[];
	candidates?: readonly CandidateLike[];
	onsave: (plan: SharedSavePlan) => void | Promise<void>;
	onsearch?: (input: MetadataSearchInput) => void | Promise<void>;
	onapplycandidate?: (input: MetadataCandidateApplyInput) => void | Promise<void>;
	disabled?: boolean;
};
	let {
		mode = 'item',
		kind = 'book',
		fields: suppliedFields,
		baseline,
		resetKey = '',
		staged = new Set<SharedFieldKey>(),
		lockedFields = [],
		candidates = [],
		onsave,
		onsearch,
		onapplycandidate,
		disabled = false
	}: MetadataFieldPickerProps = $props();

	let draft = $state<Draft>(emptyDraft());
	let storedBaseline = $state<Draft>(emptyDraft());
	let sources = $state<Partial<Record<SharedFieldKey, { candidateId: string; provider: string; text: string }>>>({});
	let loadedKey = $state('');
	let saving = $state(false);
	let saved = $state(false);
	let saveError = $state<string | null>(null);
	let validationMessage = $state<string | null>(null);
	let searchTitle = $state('');
	let searchAuthor = $state('');
	let searchIsbn = $state('');
	let searching = $state(false);
	let searchError = $state<string | null>(null);
	let candidateApplying = $state<number | null>(null);

	let fields = $derived(suppliedFields ? [...suppliedFields] : visibleFields(mode, kind));
	let currentKey = $derived(
		`${resetKey}|${mode}|${kind}|${fields.map((field) => `${field.key}=${baseline[field.key] ?? ''}`).join('\u001f')}`
	);
	let dirtyFields = $derived(changedFields(draft, storedBaseline, fields));
	let dirty = $derived(dirtyFields.length > 0);
	let validationErrors = $derived(
		fields
			.map((field) => ({ field, message: validateText(field, draft[field.key]) }))
			.filter((entry): entry is { field: SharedFieldDescriptor; message: string } => Boolean(entry.message))
	);
	let blocked = $derived(disabled || saving || searching || candidateApplying !== null);

	// A sheet stays mounted while its target changes. Reset local edits only when
	// the server baseline or scope changes; candidate refreshes must preserve edits.
	$effect(() => {
		if (!currentKey || currentKey === loadedKey) return;
		storedBaseline = { ...baseline };
		draft = { ...baseline };
		sources = {};
		validationMessage = null;
		saveError = null;
		searchError = null;
		saved = false;
		loadedKey = currentKey;
	});

	function candidateComparison(candidate: CandidateLike): ReturnType<typeof candidateOffers> {
		return candidateOffers(candidate, fields, draft);
	}

	function offersFor(field: SharedFieldDescriptor): { candidate: CandidateLike; text: string; confidence: number | null }[] {
		const offers: { candidate: CandidateLike; text: string; confidence: number | null }[] = [];
		for (const candidate of candidates) {
			const offer = candidateComparison(candidate).offers.find((entry) => entry.descriptor.key === field.key);
			if (offer) offers.push({ candidate, text: offer.text, confidence: offer.confidence });
		}
		return offers;
	}

	function setField(field: SharedFieldDescriptor, value: string): void {
		draft[field.key] = value;
		const source = sources[field.key];
		if (source && !sameValue(field.kind, source.text, value)) delete sources[field.key];
		saved = false;
		saveError = null;
		validationMessage = null;
	}

	function useCandidateField(field: SharedFieldDescriptor, candidate: CandidateLike, text: string): void {
		if (mode === 'media' && isLocked(field, lockedFields)) return;
		draft[field.key] = text;
		if (candidate.source !== 'external') {
			sources[field.key] = { candidateId: candidate.id, provider: candidate.provider, text };
		} else {
			delete sources[field.key];
		}
		saved = false;
		saveError = null;
		validationMessage = null;
	}

	function useCandidate(candidate: CandidateLike): void {
		const comparison = candidateComparison(candidate);
		for (const offer of comparison.offers) useCandidateField(offer.descriptor, candidate, offer.text);
	}

	function resetDraft(): void {
		draft = { ...storedBaseline };
		sources = {};
		saved = false;
		saveError = null;
		validationMessage = null;
	}

	async function search(): Promise<void> {
		if (!onsearch) return;
		searching = true;
		searchError = null;
		try {
			await onsearch({
				title: searchTitle.trim() || undefined,
				author: searchAuthor.trim() || undefined,
				isbn: searchIsbn.trim() || undefined,
				limit: 10
			});
		} catch (error) {
			searchError = error instanceof Error ? error.message : 'Provider search failed.';
		} finally {
			searching = false;
		}
	}

	async function applyCandidate(index: number, candidate: CandidateLike): Promise<void> {
		if (!onapplycandidate) return;
		const selectedFields = candidateComparison(candidate).offers
			.filter((offer) => !isLocked(offer.descriptor, lockedFields) && offer.descriptor.field)
			.map((offer) => offer.descriptor.field as string);
		if (!selectedFields.length) return;
		candidateApplying = index;
		searchError = null;
		try {
			await onapplycandidate({ candidateIndex: index, selectedFields });
		} catch (error) {
			searchError = error instanceof Error ? error.message : 'Unable to apply candidate.';
		} finally {
			candidateApplying = null;
		}
	}

	async function save(): Promise<void> {
		validationMessage = validationErrors[0]?.message ?? null;
		saveError = null;
		if (validationMessage) return;
		const plan = buildSharedSavePlan({ mode, fields, draft, baseline: storedBaseline, sources, lockedFields });
		if (!plan.changed.length) {
			saved = true;
			validationMessage = 'There are no metadata changes to save.';
			return;
		}
		saving = true;
		try {
			await onsave(plan);
			storedBaseline = { ...draft };
			sources = {};
			saved = true;
			validationMessage = null;
		} catch (error) {
			saveError = error instanceof Error ? error.message : 'Unable to save metadata.';
			saved = false;
		} finally {
			saving = false;
		}
	}
</script>

<Card>
	<CardHeader>
		<div class="flex flex-wrap items-start justify-between gap-3">
			<div>
				<CardTitle>Metadata</CardTitle>
				<CardDescription>
					{mode === 'media'
						? 'Edit stored library metadata. Locked fields are read-only and skipped by the server.'
						: 'Edit staged values before the item is committed. Provider choices remain explicit and auditable.'}
				</CardDescription>
			</div>
			{#if saved}
				<Badge variant="secondary">Saved</Badge>
			{:else if dirty}
				<Badge variant="outline">{dirtyFields.length} unsaved change{dirtyFields.length === 1 ? '' : 's'}</Badge>
			{/if}
		</div>
	</CardHeader>
	<CardContent class="flex flex-col gap-5">
		{#if onsearch}
			<section aria-labelledby="metadata-provider-search-heading" class="flex flex-col gap-3 rounded-lg border border-dashed p-3">
				<div>
					<h3 id="metadata-provider-search-heading" class="text-sm font-semibold">Provider search</h3>
					<p class="text-xs text-muted-foreground">Search external providers, then review and apply values explicitly.</p>
				</div>
				<div class="grid gap-3 sm:grid-cols-3">
					<div class="sm:col-span-3"><Label for="metadata-provider-search-title">Title</Label><Input id="metadata-provider-search-title" bind:value={searchTitle} placeholder="Search title" /></div>
					<div><Label for="metadata-provider-search-author">Author</Label><Input id="metadata-provider-search-author" bind:value={searchAuthor} placeholder="Author" /></div>
					<div><Label for="metadata-provider-search-isbn">ISBN</Label><Input id="metadata-provider-search-isbn" bind:value={searchIsbn} placeholder="ISBN" /></div>
					<div class="flex items-end"><Button class="w-full" variant="outline" disabled={blocked || !(searchTitle.trim() || searchAuthor.trim() || searchIsbn.trim())} onclick={() => void search()}>{searching ? 'Searching…' : 'Search providers'}</Button></div>
				</div>
				{#if searchError}<p class="text-sm text-destructive" role="alert">{searchError}</p>{/if}
			</section>
		{/if}

		{#if candidates.length}
			<section aria-labelledby="candidate-comparison-heading" class="flex flex-col gap-3">
				<div>
					<p class="text-xs text-muted-foreground">Review each proposed value against the current draft, then apply all applicable fields or choose them one at a time.</p>
				</div>
				{#each candidates as candidate, index (candidate.id)}
					{@const comparison = candidateComparison(candidate)}
					<div class="rounded-lg border bg-muted/20 p-3">
						<div class="flex flex-wrap items-center justify-between gap-2">
							<div>
								<p class="font-medium">{providerLabel(candidate.provider)}</p>
								<p class="text-xs text-muted-foreground">{candidate.providerVersion ?? 'External metadata'}{candidate.model ? ` · ${candidate.model}` : ''}</p>
							</div>
							<div class="flex items-center gap-2"><Badge variant="outline">{Math.round(candidate.confidence * 100)}% confidence</Badge><Badge variant="secondary">{candidate.status ?? 'AVAILABLE'}</Badge></div>
						</div>
						{#if comparison.offers.length}
							<div class="mt-3 flex flex-col gap-1.5">
								{#each comparison.offers as offer (offer.descriptor.key)}
									<div class="grid gap-1 text-xs sm:grid-cols-[8rem_minmax(0,1fr)]"><span class="font-medium">{offer.descriptor.label}</span><span class="min-w-0"><span class="text-muted-foreground">{draft[offer.descriptor.key] || 'Empty'}</span><span class="px-1 text-muted-foreground">→</span><strong>{offer.text}</strong>{#if isLocked(offer.descriptor, lockedFields)}<Badge variant="outline" class="ml-1">Locked</Badge>{:else if offer.applied}<Badge variant="secondary" class="ml-1">Already applied</Badge>{:else if offer.fills}<Badge variant="outline" class="ml-1">Fills empty</Badge>{:else}<Badge variant="outline" class="ml-1">Replaces draft</Badge>{/if}</span></div>
								{/each}
							</div>
							<div class="mt-3 flex flex-wrap gap-2">
								<Button size="sm" variant="outline" disabled={blocked || comparison.offers.every((offer) => isLocked(offer.descriptor, lockedFields))} onclick={() => useCandidate(candidate)}>Use applicable fields</Button>
								{#if onapplycandidate}<Button size="sm" disabled={blocked || candidateApplying !== null} onclick={() => void applyCandidate(index, candidate)}>{candidateApplying === index ? 'Applying…' : 'Apply candidate'}</Button>{/if}
							</div>
						{:else}<p class="mt-3 text-xs text-muted-foreground">No editable fields from this candidate apply to this publication.</p>{/if}
						{#if comparison.evidence.length}
							<div class="mt-3 border-t pt-2 text-xs text-muted-foreground"><p class="font-medium text-foreground">Evidence only</p><ul class="mt-1 flex flex-col gap-1">{#each comparison.evidence as evidence (evidence.key)}<li><span class="font-medium">{evidence.label}:</span>{#if evidence.url}<a class="underline underline-offset-2" href={evidence.url} target="_blank" rel="noreferrer">{evidence.text}</a>{:else}{evidence.text}{/if}</li>{/each}</ul><p class="mt-1">The backend has no mutation for these candidate fields; they are shown for review only.</p></div>
						{/if}
					</div>
				{/each}
			</section>
		{:else}<div class="rounded-lg border border-dashed p-3 text-sm text-muted-foreground">No provider candidates are available. Enter a value manually or search providers.</div>{/if}

		<section aria-labelledby="metadata-fields-heading" class="flex flex-col gap-3">
			<div><h3 id="metadata-fields-heading" class="text-sm font-semibold">Editable fields</h3><p class="text-xs text-muted-foreground">{mode === 'media' ? 'Changes use the library metadata and tag mutations supported by the server.' : 'Only staged-ingest fields are shown; file facts and unsupported provider evidence remain read-only.'}</p></div>
			{#each fields as field (field.key)}
				{@const fieldLocked = mode === 'media' && isLocked(field, lockedFields)}
				{@const fieldOffers = offersFor(field)}
				<div class="grid gap-3 rounded-lg border p-3 lg:grid-cols-[minmax(10rem,0.8fr)_minmax(0,1.6fr)] lg:items-start">
					<div><div class="flex flex-wrap items-center gap-1.5"><Label for={`metadata-${field.key}`}>{field.label}</Label>{#if fieldLocked}<Badge variant="outline">Locked</Badge>{/if}{#if mode === 'item' && staged.has(field.key)}<Badge variant="secondary">Staged</Badge>{/if}</div>{#if field.hint}<p class="mt-1 text-xs text-muted-foreground">{field.hint}</p>{/if}{#if fieldLocked}<p class="mt-1 text-xs text-muted-foreground">Unlock this field in the library policy before editing it.</p>{/if}</div>
					<div class="flex min-w-0 flex-col gap-2">
						{#if field.kind === 'textarea'}<Textarea id={`metadata-${field.key}`} value={draft[field.key]} placeholder={field.placeholder} disabled={blocked || fieldLocked} aria-invalid={Boolean(validateText(field, draft[field.key]))} oninput={(event) => setField(field, (event.currentTarget as HTMLTextAreaElement).value)} />{:else}<Input id={`metadata-${field.key}`} type={field.kind === 'integer' || field.kind === 'decimal' ? 'number' : 'text'} step={field.kind === 'decimal' ? 'any' : undefined} value={draft[field.key]} placeholder={field.placeholder} disabled={blocked || fieldLocked} aria-invalid={Boolean(validateText(field, draft[field.key]))} oninput={(event) => setField(field, (event.currentTarget as HTMLInputElement).value)} />{/if}
						{#if fieldOffers.length}<div class="flex flex-wrap items-center gap-1.5"><span class="text-xs text-muted-foreground">Use candidate:</span>{#each fieldOffers as offer (offer.candidate.id)}<Button size="sm" variant="ghost" class="h-7 px-2 text-xs" disabled={blocked || fieldLocked} onclick={() => useCandidateField(field, offer.candidate, offer.text)}>{providerLabel(offer.candidate.provider)} · {Math.round((offer.confidence ?? offer.candidate.confidence) * 100)}%</Button>{/each}</div>{/if}
						{#if mode === 'item' && !field.ingestKey}<p class="text-xs text-muted-foreground">This field has no staged-ingest mutation.</p>{/if}
					</div>
				</div>
			{/each}
		</section>

		{#if validationErrors.length}<p class="text-sm text-destructive" role="alert">{validationErrors[0].message}</p>{:else if validationMessage}<p class="text-sm text-muted-foreground" role="status">{validationMessage}</p>{/if}
		{#if saveError}<p class="text-sm text-destructive" role="alert">{saveError}</p>{/if}
		<div class="flex flex-wrap items-center justify-end gap-2 border-t pt-4"><Button variant="ghost" disabled={blocked || !dirty} onclick={resetDraft}>Cancel</Button><Button disabled={blocked || !dirty || validationErrors.length > 0} onclick={() => void save()}>{#if saving}Saving…{:else if saved}Saved{:else}Save metadata{/if}</Button></div>
	</CardContent>
</Card>
