<script lang="ts">
	import { Button } from '$lib/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '$lib/components/ui/card';
	import { Input } from '$lib/components/ui/input';
	import { Label } from '$lib/components/ui/label';
	import { Textarea } from '$lib/components/ui/textarea';
	import {
		METADATA_FIELDS,
		humanize,
		isApplyableMetadataField,
		parseJsonObject
	} from '$lib/ingest/helpers';
	import type {
		IngestDropItemFieldsFragment,
		IngestMetadataFieldMode,
		IngestMetadataFieldSelectionInput,
		MetadataField
	} from '$lib/graphql/generated/graphql';

	type Candidate = IngestDropItemFieldsFragment['metadataCandidates'][number];
	type Draft = {
		mode: IngestMetadataFieldMode;
		candidateId: string;
		manualValue: string;
	};

	let {
		candidates,
		onsave,
		disabled = false
	}: {
		candidates: Candidate[];
		onsave: (selections: IngestMetadataFieldSelectionInput[]) => void;
		disabled?: boolean;
	} = $props();

	function draftFor(field: MetadataField): Draft {
		return { mode: 'KEEP_EXISTING', candidateId: '', manualValue: '' };
	}

	let drafts = $state<Record<MetadataField, Draft>>(
		Object.fromEntries(METADATA_FIELDS.map((field) => [field, draftFor(field)])) as Record<
			MetadataField,
			Draft
		>
	);
	let validationMessage = $state<string | null>(null);

	function candidatesFor(field: MetadataField): Candidate[] {
		return candidates.filter((candidate) => Object.hasOwn(parseJsonObject(candidate.fields), field));
	}

	function candidateValue(candidate: Candidate, field: MetadataField): string {
		const value = parseJsonObject(candidate.fields)[field];
		return typeof value === 'string' ? value : JSON.stringify(value);
	}

	function updateMode(field: MetadataField, mode: IngestMetadataFieldMode): void {
		drafts[field].mode = mode;
		if (mode !== 'CANDIDATE') drafts[field].candidateId = '';
		if (mode !== 'MANUAL') drafts[field].manualValue = '';
	}

	function updateCandidate(field: MetadataField, candidateId: string): void {
		drafts[field].candidateId = candidateId;
	}

	function updateManualValue(field: MetadataField, value: string): void {
		drafts[field].manualValue = value;
	}
	function buildSelections(): IngestMetadataFieldSelectionInput[] | null {
		const selections: IngestMetadataFieldSelectionInput[] = [];
		for (const field of METADATA_FIELDS) {
			if (!isApplyableMetadataField(field)) continue;
			const draft = drafts[field];
			if (draft.mode === 'CANDIDATE' && !draft.candidateId) {
				validationMessage = `${humanize(field)} needs a candidate.`;
				return null;
			}
			if (draft.mode === 'MANUAL') {
				if (!draft.manualValue.trim()) {
					validationMessage = `${humanize(field)} needs a value.`;
					return null;
				}
				let value: unknown;
				try {
					value = JSON.parse(draft.manualValue);
				} catch {
					value = draft.manualValue;
				}
				selections.push({ field, mode: draft.mode, value });
				continue;
			}
			if (draft.mode === 'CANDIDATE') {
				selections.push({ field, mode: draft.mode, candidateId: draft.candidateId });
				continue;
			}
			selections.push({ field, mode: draft.mode });
		}
		validationMessage = null;
		return selections;
	}

	function save(): void {
		const selections = buildSelections();
		if (selections) onsave(selections);
	}
</script>

<Card>
	<CardHeader>
		<CardTitle>Field picks</CardTitle>
		<CardDescription>
			Choose KEEP, a provider candidate, a manual JSON value, or CLEAR for every metadata field.
		</CardDescription>
	</CardHeader>
	<CardContent class="flex flex-col gap-3">
		{#each METADATA_FIELDS as field (field)}
			{@const fieldCandidates = candidatesFor(field)}
			{@const applyable = isApplyableMetadataField(field)}
			<div class="grid gap-2 rounded-lg border p-3 lg:grid-cols-[minmax(9rem,0.7fr)_10rem_minmax(12rem,1fr)] lg:items-start">
				<div>
					<Label for={`mode-${field}`}>{humanize(field)}</Label>
					{#if !applyable}
						<p class="mt-1 text-xs text-muted-foreground">Evidence only; this field cannot be applied.</p>
					{:else if fieldCandidates.length}
						<p class="mt-1 text-xs text-muted-foreground">
							{fieldCandidates.length} candidate{fieldCandidates.length === 1 ? '' : 's'} available
						</p>
					{/if}
				</div>
				<select
					id={`mode-${field}`}
					class="h-9 rounded-md border bg-background px-3 text-sm"
					value={drafts[field].mode}
					disabled={disabled || !applyable}
					onchange={(event) =>
						updateMode(field, (event.currentTarget as HTMLSelectElement).value as IngestMetadataFieldMode)}
				>
					<option value="KEEP_EXISTING">Keep existing</option>
					<option value="CANDIDATE" disabled={!fieldCandidates.length}>Candidate</option>
					<option value="MANUAL">Manual</option>
					<option value="CLEAR">Clear</option>
				</select>
				{#if applyable && drafts[field].mode === 'CANDIDATE'}
					<select
						class="h-9 rounded-md border bg-background px-3 text-sm"
						value={drafts[field].candidateId}
						disabled={disabled}
						onchange={(event) =>
							updateCandidate(field, (event.currentTarget as HTMLSelectElement).value)}
					>
						<option value="">Choose candidate</option>
						{#each fieldCandidates as candidate}
							<option value={candidate.id}>
								{candidate.provider} · {Math.round(candidate.confidence * 100)}% · {candidateValue(candidate, field)}
							</option>
						{/each}
					</select>
				{:else if applyable && drafts[field].mode === 'MANUAL'}
					<Textarea
						value={drafts[field].manualValue}
						placeholder='JSON value, e.g. "A title"'
						disabled={disabled}
						oninput={(event) => updateManualValue(field, (event.currentTarget as HTMLTextAreaElement).value)}
					/>
				{:else}
					<Input value={drafts[field].mode === 'CLEAR' ? 'Value will be cleared' : 'No change'} disabled />
				{/if}
			</div>
		{/each}
		{#if validationMessage}
			<p class="text-sm text-destructive" role="alert">{validationMessage}</p>
		{/if}
		<Button class="self-end" disabled={disabled} onclick={save}>Save field picks</Button>
	</CardContent>
</Card>
