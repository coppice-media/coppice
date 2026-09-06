<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { Button } from '@stump/ui/components/ui/button';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import * as Select from '@stump/ui/components/ui/select';
	import type { EntityScope } from './scope.svelte';

	let {
		scope,
		title,
		description,
		searchPlaceholder
	}: {
		scope: EntityScope;
		title: string;
		description: string;
		searchPlaceholder: string;
	} = $props();

	const LINKS = [
		{ href: resolve('/entities/authors'), label: 'Authors' },
		{ href: resolve('/entities/publishers'), label: 'Publishers' },
		{ href: resolve('/entities/tags'), label: 'Tags' }
	];

	// Keep the selected library when switching between the three grids.
	const suffix = $derived(scope.libraryId ? `?library=${encodeURIComponent(scope.libraryId)}` : '');

	let draft = $state('');
	$effect(() => {
		draft = scope.search ?? '';
	});

	function submit(event: SubmitEvent): void {
		event.preventDefault();
		scope.setSearch(draft.trim() ? draft.trim() : null);
	}
</script>

<div class="flex flex-col gap-4">
	<div>
		<h1 class="text-2xl font-semibold tracking-tight">{title}</h1>
		<p class="text-sm text-muted-foreground">{description}</p>
	</div>

	<div class="flex flex-wrap items-end gap-3">
		<nav aria-label="Entity grids" class="flex items-center gap-1 text-sm">
			{#each LINKS as link (link.label)}
				<a
					class="rounded-md px-3 py-2 hover:bg-muted aria-[current=page]:bg-muted aria-[current=page]:font-medium"
					href={`${link.href}${suffix}`}
					aria-current={page.url.pathname === link.href ? 'page' : undefined}
				>
					{link.label}
				</a>
			{/each}
		</nav>

		<div class="flex flex-col gap-1">
			<Label class="text-xs text-muted-foreground" for="entity-library">Library</Label>
			<Select.Root
				type="single"
				value={scope.libraryId ?? ''}
				onValueChange={(value) => scope.setLibrary(value)}
				disabled={scope.libraries.length < 2}
			>
				<Select.Trigger id="entity-library" class="w-52">
					{scope.library?.name ?? 'No libraries yet'}
				</Select.Trigger>
				<Select.Content>
					{#each scope.libraries as library (library.id)}
						<Select.Item value={library.id} label={library.name} />
					{/each}
				</Select.Content>
			</Select.Root>
		</div>

		<form class="flex items-end gap-2" onsubmit={submit}>
			<div class="flex flex-col gap-1">
				<Label class="text-xs text-muted-foreground" for="entity-search">Name contains</Label>
				<Input
					id="entity-search"
					class="w-48"
					bind:value={draft}
					placeholder={searchPlaceholder}
				/>
			</div>
			<Button size="sm" variant="outline" type="submit">Search</Button>
		</form>
	</div>
</div>
