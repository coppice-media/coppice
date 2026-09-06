<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import type { ResultOf } from '@graphql-typed-document-node/core';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Input } from '@stump/ui/components/ui/input';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { request } from '@stump/ui/graphql/client';
	import {
		ProviderSourceHealthRowsDocument,
		ProviderSourcesDocument,
		SetProviderSourceHeadersDocument
	} from '$lib/graphql/generated/graphql';

	const queryClient = useQueryClient();

	type ProviderSource = ResultOf<typeof ProviderSourcesDocument>['providerSources'][number];
	type HealthRow = ResultOf<typeof ProviderSourceHealthRowsDocument>['providerSourceHealth'][number];
	type HeaderDraft = { name: string; value: string };

	/// A challenged source needs exactly these two: the clearance cookie and
	/// the user agent it was issued to.
	const DEFAULT_HEADERS: HeaderDraft[] = [
		{ name: 'Cookie', value: '' },
		{ name: 'User-Agent', value: '' }
	];

	let drafts = $state<Record<string, HeaderDraft[]>>({});
	let saved = $state<Record<string, string>>({});

	const sourcesQuery = createQuery(() => ({
		queryKey: ['provider-sources'],
		queryFn: () => request(ProviderSourcesDocument, {}),
		enabled: browser
	}));

	const healthQuery = createQuery(() => ({
		queryKey: ['provider-source-health'],
		queryFn: () => request(ProviderSourceHealthRowsDocument, {}),
		enabled: browser
	}));

	const saveMutation = createMutation(() => ({
		mutationFn: (input: { instanceId: string; headers: Record<string, string> }) =>
			request(SetProviderSourceHeadersDocument, { instanceId: input.instanceId, headers: input.headers }),
		onSuccess: (result, variables) => {
			const stored = result.setProviderSourceHeaders;
			saved[variables.instanceId] = stored.length
				? `Saved ${stored.length} header${stored.length > 1 ? 's' : ''}.`
				: 'Cleared every configured header.';
			delete drafts[variables.instanceId];
			void queryClient.invalidateQueries({ queryKey: ['provider-sources'] });
		},
		onError: (error, variables) => {
			saved[variables.instanceId] = error instanceof Error ? error.message : 'The server refused the headers.';
		}
	}));

	let sources = $derived(sourcesQuery.data?.providerSources ?? []);

	function normalise(url: string): string {
		return url.replace(/\/+$/, '');
	}

	/// The health row for the source's host: rows are keyed by catalog source
	/// id, so a source instance is matched the way the health job groups its
	/// targets — by base URL.
	function healthFor(source: ProviderSource): HealthRow | undefined {
		return healthQuery.data?.providerSourceHealth.find(
			(row) => normalise(row.baseUrl) === normalise(source.baseUrl)
		);
	}

	function isChallenged(row: HealthRow | undefined): boolean {
		return row?.challenged ?? false;
	}

	function draftFor(source: ProviderSource): HeaderDraft[] {
		const existing = drafts[source.id];
		if (existing) return existing;
		const fromServer = source.requestHeaders.map((header) => ({ name: header.name, value: '' }));
		return fromServer.length ? fromServer : DEFAULT_HEADERS.map((header) => ({ ...header }));
	}

	function updateDraft(source: ProviderSource, index: number, patch: Partial<HeaderDraft>): void {
		const rows = draftFor(source).map((row, position) => (position === index ? { ...row, ...patch } : row));
		drafts = { ...drafts, [source.id]: rows };
	}

	function addRow(source: ProviderSource): void {
		drafts = { ...drafts, [source.id]: [...draftFor(source), { name: '', value: '' }] };
	}

	function save(source: ProviderSource): void {
		const headers: Record<string, string> = {};
		for (const row of draftFor(source)) {
			if (row.name.trim() && row.value.trim()) headers[row.name.trim()] = row.value.trim();
		}
		saveMutation.mutate({ instanceId: source.id, headers });
	}

	function clearAll(source: ProviderSource): void {
		saveMutation.mutate({ instanceId: source.id, headers: {} });
	}
</script>

<svelte:head><title>Settings · Remote sources</title></svelte:head>

<div class="flex flex-col gap-6">
	<div>
		<p class="text-sm font-medium text-primary">Provider host</p>
		<h1 class="text-3xl font-semibold tracking-tight">Remote sources</h1>
		<p class="mt-1 max-w-3xl text-muted-foreground">
			Request headers sent with every request one source makes. Use them for a source behind a Cloudflare challenge:
			copy the <code>cf_clearance</code> cookie <em>and</em> the exact User-Agent from a browser that solved the
			challenge. Values are write-only — the server only ever reports a masked preview, and a clearance cookie expires.
		</p>
	</div>
	{#if sourcesQuery.isPending}
		{#each Array(2) as _, index (index)}
			<Skeleton class="h-40 w-full" />
		{/each}
	{:else if sourcesQuery.isError}
		<p class="text-sm text-destructive">
			{sourcesQuery.error instanceof Error ? sourcesQuery.error.message : 'Unable to load provider sources.'}
		</p>
	{:else if !sources.length}
		<p class="text-sm text-muted-foreground">
			No source instances are enabled. Enable one with <code>enableProviderSource</code> first.
		</p>
	{:else}
		{#each sources as source (source.id)}
			{@const health = healthFor(source)}
			<Card>
				<CardHeader>
					<div class="flex flex-wrap items-start justify-between gap-4">
						<div>
							<CardTitle class="flex items-center gap-2">
								{source.name}
								<Badge variant={source.enabled ? 'secondary' : 'outline'}>
									{source.enabled ? 'Enabled' : 'Disabled'}
								</Badge>
								{#if isChallenged(health)}
									<Badge variant="destructive">Cloudflare challenge</Badge>
								{:else if health}
									<Badge variant="outline">{health.status}</Badge>
								{/if}
							</CardTitle>
							<CardDescription>
								{source.id} · {source.lang} · {source.baseUrl}
								{#if health?.error && !isChallenged(health)}· {health.error}{/if}
							</CardDescription>
						</div>
						{#if source.requestHeaders.length}
							<div class="flex flex-col items-end gap-1 text-xs text-muted-foreground">
								{#each source.requestHeaders as header (header.name)}
									<span class="tabular-nums">{header.name}: {header.preview} ({header.length} chars)</span>
								{/each}
							</div>
						{/if}
					</div>
				</CardHeader>
				<CardContent class="flex flex-col gap-3">
					<p class="text-sm font-medium">Request headers</p>
					{#each draftFor(source) as row, index (index)}
						<div class="flex flex-wrap items-center gap-2">
							<Input
								class="max-w-56"
								placeholder="Header name"
								value={row.name}
								oninput={(event) => updateDraft(source, index, { name: (event.currentTarget as HTMLInputElement).value })}
							/>
							<Input
								class="max-w-md"
								type="password"
								autocomplete="off"
								placeholder={source.requestHeaders.some((header) => header.name === row.name.trim().toLowerCase())
									? 'Configured; re-enter to keep it'
									: 'Value'}
								value={row.value}
								oninput={(event) => updateDraft(source, index, { value: (event.currentTarget as HTMLInputElement).value })}
							/>
						</div>
					{/each}
					<p class="text-xs text-muted-foreground">
						Saving replaces every header for this source, so re-enter the ones you want to keep.
					</p>
					{#if saved[source.id]}
						<p class="text-xs text-muted-foreground">{saved[source.id]}</p>
					{/if}
					<div class="flex flex-wrap justify-end gap-2">
						<Button variant="outline" onclick={() => addRow(source)}>Add header</Button>
						<Button
							variant="outline"
							disabled={!source.requestHeaders.length || saveMutation.isPending}
							onclick={() => clearAll(source)}
						>
							Clear all
						</Button>
						<Button disabled={saveMutation.isPending} onclick={() => save(source)}>
							{saveMutation.isPending ? 'Saving…' : 'Save headers'}
						</Button>
					</div>
				</CardContent>
			</Card>
		{/each}
	{/if}
</div>
