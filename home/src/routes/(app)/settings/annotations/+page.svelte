<script lang="ts">
	/**
	 * `/settings/annotations` — the annotation export sinks.
	 *
	 * The server compiles in a fixed set of sinks (`markdown`, `git`), each
	 * publishing its own `IngestSettingDefinition` schema, so the forms below
	 * are built from that schema rather than hard-coded. Values flagged
	 * `secret` are encrypted at rest and never rendered back: an empty secret
	 * field means "keep what is stored".
	 */
	import { browser } from '$app/environment';
	import { resolve } from '$app/paths';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import ArrowLeftIcon from '@lucide/svelte/icons/arrow-left';
	import UploadIcon from '@lucide/svelte/icons/upload';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import {
		Card,
		CardContent,
		CardDescription,
		CardFooter,
		CardHeader,
		CardTitle
	} from '@stump/ui/components/ui/card';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Input } from '@stump/ui/components/ui/input';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { Switch } from '@stump/ui/components/ui/switch';
	import { request } from '@stump/ui/graphql/client';
	import {
		ConsoleAnnotationSinksDocument,
		ConsoleRunAnnotationSyncDocument,
		ConsoleSetAnnotationSinkSettingsDocument,
		type IngestSettingValueType
	} from '$lib/graphql/generated/graphql';
	import { absoluteTime, relativeTime } from '$lib/format';

	const queryClient = useQueryClient();

	const sinksQuery = createQuery(() => ({
		queryKey: ['annotation-sinks'],
		queryFn: () => request(ConsoleAnnotationSinksDocument, {}),
		enabled: browser
	}));
	const sinks = $derived(sinksQuery.data?.annotationSinks ?? []);
	const status = $derived(sinksQuery.data?.annotationSyncStatus ?? null);

	/** Draft values per sink, keyed by setting key. Secrets start empty. */
	let drafts = $state<Record<string, Record<string, string>>>({});
	let seeded = $state(false);
	let savingSink = $state<string | null>(null);

	// The query returns the sink schemas but never the stored values (they may
	// contain secrets), so the forms are seeded once with each setting's
	// default and left to the user from then on.
	$effect(() => {
		if (!sinksQuery.data || seeded) return;
		seeded = true;
		drafts = Object.fromEntries(
			sinksQuery.data.annotationSinks.map((sink) => [
				sink.id,
				Object.fromEntries(
					sink.settings.map((definition) => [
						definition.key,
						definition.secret ? '' : stringify(definition.defaultValue)
					])
				)
			])
		);
	});

	const saveSink = createMutation(() => ({
		mutationFn: (variables: { sinkId: string; enabled: boolean }) =>
			request(ConsoleSetAnnotationSinkSettingsDocument, {
				sinkId: variables.sinkId,
				settings: serialize(variables.sinkId),
				enabled: variables.enabled
			}),
		onSuccess: () => {
			void queryClient.invalidateQueries({ queryKey: ['annotation-sinks'] });
			toast.success('Sink settings saved.');
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to save the sink settings.'),
		onSettled: () => (savingSink = null)
	}));

	const runSync = createMutation(() => ({
		mutationFn: () => request(ConsoleRunAnnotationSyncDocument, {}),
		onSuccess: () => {
			void queryClient.invalidateQueries({ queryKey: ['annotation-sinks'] });
			toast.success('Export queued.');
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'The export could not be queued.')
	}));

	function stringify(value: unknown): string {
		if (value === null || value === undefined) return '';
		if (typeof value === 'string') return value;
		if (typeof value === 'number' || typeof value === 'boolean') return String(value);
		return JSON.stringify(value);
	}

	function parse(value: string, type: IngestSettingValueType): unknown {
		if (type === 'BOOLEAN') return value === 'true';
		if (type === 'INTEGER') return Number.parseInt(value, 10);
		if (type === 'NUMBER') return Number(value);
		if (type === 'JSON') {
			try {
				return JSON.parse(value);
			} catch {
				return value;
			}
		}
		return value;
	}

	/**
	 * Only the sink's own keys, and only secrets the user actually typed: the
	 * mutation replaces the settings map, and an omitted secret keeps the
	 * encrypted value already stored.
	 */
	function serialize(sinkId: string): Record<string, unknown> {
		const sink = sinks.find((candidate) => candidate.id === sinkId);
		const draft = drafts[sinkId] ?? {};
		return Object.fromEntries(
			(sink?.settings ?? [])
				.filter((definition) => !(definition.secret && (draft[definition.key] ?? '') === ''))
				.map((definition) => [
					definition.key,
					parse(draft[definition.key] ?? '', definition.valueType)
				])
		);
	}

	function setDraft(sinkId: string, key: string, value: string): void {
		drafts[sinkId] = { ...(drafts[sinkId] ?? {}), [key]: value };
	}

	function stateOf(sinkId: string) {
		return status?.sinks.find((sink) => sink.sinkId === sinkId) ?? null;
	}

	function save(sinkId: string, enabled: boolean): void {
		savingSink = sinkId;
		saveSink.mutate({ sinkId, enabled });
	}
</script>

<svelte:head>
	<title>Annotation export · Stump</title>
</svelte:head>

<div class="flex flex-col gap-6">
	<div class="flex flex-wrap items-start gap-3">
		<div class="mr-auto">
			<h1 class="text-2xl font-semibold tracking-tight">Annotation export</h1>
			<p class="mt-1 max-w-2xl text-sm text-muted-foreground">
				Project your highlights, notes, bookmarks, and reading summaries into a markdown vault or
				a git repository — one file per book. An export runs shortly after you change anything,
				and only for the sinks you enable here.
			</p>
		</div>
		<Button size="sm" variant="ghost" href={resolve('/(app)/annotations')}>
			<ArrowLeftIcon aria-hidden="true" />
			Annotations
		</Button>
		<Button
			size="sm"
			variant="outline"
			disabled={runSync.isPending}
			onclick={() => runSync.mutate()}
		>
			<UploadIcon aria-hidden="true" />
			{runSync.isPending ? 'Queueing…' : 'Export now'}
		</Button>
	</div>

	{#if sinksQuery.isPending}
		<div class="flex flex-col gap-4">
			{#each { length: 2 } as _, index (index)}
				<Skeleton class="h-52 w-full rounded-xl" />
			{/each}
		</div>
	{:else if sinksQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load export sinks</AlertTitle>
			<AlertDescription>
				{sinksQuery.error instanceof Error
					? sinksQuery.error.message
					: 'The server did not return annotation sinks.'}
			</AlertDescription>
		</Alert>
	{:else if sinks.length === 0}
		<Empty>
			<EmptyHeader>
				<EmptyTitle>No export sinks</EmptyTitle>
				<EmptyDescription>
					This server build ships no annotation sinks, so there is nothing to export to.
				</EmptyDescription>
			</EmptyHeader>
		</Empty>
	{:else}
		{#if status?.pending}
			<Alert>
				<AlertTitle>Export pending</AlertTitle>
				<AlertDescription>
					A debounced export is scheduled for your account and runs once your annotations settle.
				</AlertDescription>
			</Alert>
		{/if}

		{#each sinks as sink (sink.id)}
			{@const state = stateOf(sink.id)}
			<Card>
				<CardHeader class="flex flex-wrap items-start gap-2">
					<div class="mr-auto">
						<CardTitle class="text-base">{sink.name}</CardTitle>
						<CardDescription>{sink.description}</CardDescription>
					</div>
					<Badge variant={state?.enabled ? 'default' : 'outline'}>
						{state?.enabled ? 'Enabled' : 'Disabled'}
					</Badge>
					<Badge variant="secondary">{sink.id}</Badge>
				</CardHeader>
				<CardContent class="flex flex-col gap-5">
					{#if state?.lastError}
						<Alert variant="destructive">
							<AlertTitle>Last export failed</AlertTitle>
							<AlertDescription>{state.lastError}</AlertDescription>
						</Alert>
					{/if}
					<p class="text-xs text-muted-foreground">
						{#if state?.lastRunAt}
							Last run {relativeTime(state.lastRunAt)} ({absoluteTime(state.lastRunAt)})
						{:else}
							Never run.
						{/if}
					</p>

					{#if sink.settings.length === 0}
						<p class="text-sm text-muted-foreground">This sink needs no configuration.</p>
					{:else}
						<div class="grid gap-5 md:grid-cols-2">
							{#each sink.settings as definition (definition.key)}
								{@const id = `${sink.id}-${definition.key}`}
								<div class="grid gap-2 text-sm">
									<label class="font-medium" for={id}>
										{definition.label}
										{#if definition.required}
											<span class="text-destructive" aria-hidden="true">*</span>
										{/if}
									</label>
									{#if definition.valueType === 'BOOLEAN'}
										<Switch
											{id}
											checked={drafts[sink.id]?.[definition.key] === 'true'}
											onCheckedChange={(next) =>
												setDraft(sink.id, definition.key, next ? 'true' : 'false')}
										/>
									{:else}
										<Input
											{id}
											type={definition.secret ? 'password' : 'text'}
											autocomplete="off"
											placeholder={definition.secret
												? 'Stored — leave blank to keep'
												: stringify(definition.defaultValue)}
											value={drafts[sink.id]?.[definition.key] ?? ''}
											oninput={(event) =>
												setDraft(sink.id, definition.key, event.currentTarget.value)}
										/>
									{/if}
									{#if definition.description}
										<span class="text-xs text-muted-foreground">
											{definition.description}
											{#if definition.helpUrl}
												<a
													class="underline"
													href={definition.helpUrl}
													target="_blank"
													rel="noreferrer noopener">Docs</a
												>
											{/if}
										</span>
									{/if}
								</div>
							{/each}
						</div>
					{/if}
				</CardContent>
				<CardFooter class="flex flex-wrap gap-2">
					<Button
						size="sm"
						disabled={savingSink === sink.id}
						onclick={() => save(sink.id, true)}
					>
						{savingSink === sink.id ? 'Saving…' : state?.enabled ? 'Save settings' : 'Enable'}
					</Button>
					{#if state?.enabled}
						<Button
							size="sm"
							variant="outline"
							disabled={savingSink === sink.id}
							onclick={() => save(sink.id, false)}
						>
							Disable
						</Button>
					{/if}
				</CardFooter>
			</Card>
		{/each}
	{/if}
</div>
