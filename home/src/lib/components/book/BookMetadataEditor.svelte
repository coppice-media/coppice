<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Separator } from '@stump/ui/components/ui/separator';
	import MetadataFieldPicker from '@stump/ui/components/metadata/FieldPicker.svelte';
	import {
		draftFromMetadata,
		isLocked,
		metadataInputFromDraft,
		visibleFields,
		type Draft,
		type SharedFieldDescriptor,
		type SharedPublicationKind,
		type SharedSavePlan
	} from '@stump/ui/editor-fields';
	import {
		metadataSearchCandidateToShared,
		type MetadataSearchCandidate
	} from '@stump/ui/graphql/metadata';
	export type MetadataScope = 'WORK' | 'EBOOK' | 'AUDIOBOOK' | 'BOTH';
	type Metadata = Record<string, unknown> & { lockedFields?: string[] | null };
	type Edition = { mediaId: string; kind: 'EBOOK' | 'AUDIOBOOK' | 'OTHER'; metadata?: Metadata | null };
	type WorkMetadata = { title?: string | null; author?: string | null; metadata?: Record<string, unknown> | null; lockedFields?: string[] | null };
	type Candidate = {
		provider: string;
		externalId: string;
		confidence: number;
		metadata?: Record<string, unknown> | null;
	};

	let {
		edition,
		audiobookEdition = null,
		workMetadata = null,
		candidates = [],
		canEdit = false,
		canSearch = false,
		canApplyCandidate = false,
		loading = false,
		onsearch,
		onapply,
		onapplyCandidate
	}: {
		edition: Edition;
		audiobookEdition?: Edition | null;
		workMetadata?: WorkMetadata | null;
		candidates?: Candidate[];
		canEdit?: boolean;
		canSearch?: boolean;
		canApplyCandidate?: boolean;
		loading?: boolean;
		onsearch: (input: { title?: string; author?: string; isbn?: string; limit?: number }) => void | Promise<void>;
		onapply: (input: { scope: MetadataScope; selectedFields: string[]; metadata: Record<string, unknown> }) => void | Promise<void>;
		onapplyCandidate?: (input: { candidateIndex: number; scope: MetadataScope; selectedFields: string[] }) => void | Promise<void>;
	} = $props();

	let scope = $state<MetadataScope>('EBOOK');
	let loadedScope = $state<MetadataScope | null>(null);

	const hasAudiobook = $derived(Boolean(audiobookEdition));
	const targetEdition = $derived(
		scope === 'AUDIOBOOK' ? audiobookEdition : edition
	);
	const targetKind = $derived<SharedPublicationKind>(targetEdition?.kind === 'AUDIOBOOK' ? 'audiobook' : 'book');
	const targetMetadata = $derived(
		scope === 'WORK'
			? {
					...(workMetadata?.metadata ?? {}),
					title: workMetadata?.title ?? workMetadata?.metadata?.title,
					writers: workMetadata?.author ?? workMetadata?.metadata?.writers
				}
			: (targetEdition?.metadata ?? {})
	);
	const baseLockedFields = $derived(
		(scope === 'WORK' ? workMetadata?.lockedFields : targetEdition?.metadata?.lockedFields) ?? []
	);
	// Home's metadata mutation has no tags input. Keeping TAGS visible but
	// policy-locked preserves the shared field matrix without sending an
	// unsupported selection to the resolver.
	const lockedFields = $derived([...baseLockedFields, 'TAGS']);
	const fields = $derived(visibleFields('media', targetKind));
	const baseline = $derived<Draft>(draftFromMetadata(targetMetadata));
	const scopeLabel = $derived(scope === 'WORK' ? 'Work' : scope === 'BOTH' ? 'Both editions' : scope === 'EBOOK' ? 'Ebook' : 'Audiobook');
	const scopeOptions = $derived.by(() => {
		const options: { value: MetadataScope; label: string; description: string }[] = [
			{ value: 'WORK', label: 'Work', description: 'Shared identity and work-level fields' },
			{ value: 'EBOOK', label: 'Ebook', description: 'Only the ebook edition' }
		];
		if (hasAudiobook) {
			options.push({ value: 'AUDIOBOOK', label: 'Audiobook', description: 'Only the audiobook edition' });
			options.push({ value: 'BOTH', label: 'Both', description: 'Write changed fields to both editions' });
		}
		return options;
	});

	$effect(() => {
		if (loadedScope === null) {
			scope = edition.kind === 'AUDIOBOOK' ? 'AUDIOBOOK' : 'EBOOK';
			loadedScope = scope;
			return;
		}
		if (scope === loadedScope) return;
		if (scope === 'AUDIOBOOK' && !hasAudiobook) scope = 'EBOOK';
		loadedScope = scope;
	});

	const sharedCandidates = $derived(
		candidates.map((candidate) =>
			metadataSearchCandidateToShared(candidate as unknown as MetadataSearchCandidate)
		)
	);

	function savePlan(plan: SharedSavePlan): Promise<void> {
		const selectedFields = plan.changed
			.map((key) => plan.fields.find((field) => field.key === key))
			.filter((field): field is SharedFieldDescriptor => field !== undefined && field.field !== null && !isLocked(field, lockedFields))
			.map((field) => field.field as string);
		return Promise.resolve(
			onapply({
				scope,
				selectedFields,
				metadata: metadataInputFromDraft(plan.draft)
			})
		);
	}

	function applyCandidate(input: { candidateIndex: number; selectedFields: string[] }): void | Promise<void> {
		if (!onapplyCandidate) return;
		return onapplyCandidate({ ...input, scope });
	}
</script>

<Card>
	<CardHeader>
		<div class="flex flex-wrap items-start justify-between gap-3">
			<div>
				<CardTitle>Edit metadata</CardTitle>
				<CardDescription>Choose a destination, review provider values, then save changed fields explicitly. Locked fields remain read-only.</CardDescription>
			</div>
			<Badge variant="outline">{scopeLabel}</Badge>
		</div>
	</CardHeader>
	<CardContent class="flex flex-col gap-6">
		<section aria-labelledby="metadata-destination-heading" class="flex flex-col gap-3">
			<div>
				<h3 id="metadata-destination-heading" class="text-sm font-semibold">Destination</h3>
				<p class="text-xs text-muted-foreground">The server writes only the selected destination; no values are mirrored implicitly.</p>
			</div>
			<div class="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
				{#each scopeOptions as option (option.value)}
					<button type="button" class="rounded-lg border p-3 text-left transition hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring {scope === option.value ? 'border-primary bg-primary/10' : ''}" onclick={() => (scope = option.value)} aria-pressed={scope === option.value}>
						<span class="flex items-center gap-2 text-sm font-medium">{#if scope === option.value}<CheckIcon class="size-4 text-primary" aria-hidden="true" />{/if}{option.label}</span>
						<span class="mt-1 block text-xs text-muted-foreground">{option.description}</span>
					</button>
				{/each}
			</div>
		</section>

		<Separator />
		<MetadataFieldPicker
			mode="media"
			kind={targetKind}
			fields={fields}
			baseline={baseline}
			resetKey={`${edition.mediaId}:${audiobookEdition?.mediaId ?? ''}:${scope}`}
			lockedFields={lockedFields}
			candidates={sharedCandidates}
			onsave={savePlan}
			onsearch={canSearch ? onsearch : undefined}
			onapplycandidate={canApplyCandidate && onapplyCandidate ? applyCandidate : undefined}
			disabled={!canEdit || loading}
		/>
	</CardContent>
</Card>
