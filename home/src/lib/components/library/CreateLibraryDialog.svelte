<script lang="ts">
	import { createMutation, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Button } from '@stump/ui/components/ui/button';
	import * as Dialog from '@stump/ui/components/ui/dialog';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import * as Select from '@stump/ui/components/ui/select';
	import { Switch } from '@stump/ui/components/ui/switch';
	import { request } from '@stump/ui/graphql/client';
	import {
		ConsoleCreateLibraryDocument,
		type LibraryPattern,
		type LibraryType
	} from '$lib/graphql/generated/graphql';
	import { LIBRARY_PATTERN_LABELS, LIBRARY_TYPE_LABELS } from '$lib/library';

	let { open = $bindable(false) }: { open?: boolean } = $props();

	let name = $state('');
	let path = $state('');
	let description = $state('');
	let libraryType = $state<LibraryType>('MIXED');
	let libraryPattern = $state<LibraryPattern>('SERIES_BASED');
	let watch = $state(true);
	let scanAfterPersist = $state(true);

	const queryClient = useQueryClient();
	const create = createMutation(() => ({
		mutationFn: () =>
			request(ConsoleCreateLibraryDocument, {
				input: {
					name: name.trim(),
					path: path.trim(),
					description: description.trim() ? description.trim() : null,
					scanAfterPersist,
					// `LibraryConfigInput` has no optional flags, so the dialog
					// sends explicit defaults for fields this dialog does not
					// expose alongside the three choices the user can make.
					config: {
						libraryType,
						libraryPattern,
						watch,
						convertRarToZip: false,
						hardDeleteConversions: false,
						generateFileHashes: false,
						generateKoreaderHashes: false,
						processMetadata: true,
						defaultLibraryViewMode: 'SERIES',
						hideSeriesView: false,
						skipBookOverview: false,
						processThumbnailColorsEvenWithoutConfig: false,
						defaultReadingDir: 'LTR',
						defaultReadingMode: 'PAGED',
						defaultReadingImageScaleFit: 'HEIGHT'
					}
				}
			}),
		onSuccess: (result) => {
			toast.success(
				scanAfterPersist
					? `Created ${result.createLibrary.name}; the first scan is queued.`
					: `Created ${result.createLibrary.name}.`
			);
			void queryClient.invalidateQueries({ queryKey: ['libraries'] });
			open = false;
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to create the library.')
	}));

	const canSubmit = $derived(name.trim().length > 0 && path.trim().length > 0);

	function submit(event: SubmitEvent): void {
		event.preventDefault();
		if (canSubmit) create.mutate();
	}

	$effect(() => {
		if (open) return;
		name = '';
		path = '';
		description = '';
		libraryType = 'MIXED';
		libraryPattern = 'SERIES_BASED';
		watch = true;
		scanAfterPersist = true;
	});
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="sm:max-w-xl">
		<Dialog.Header>
			<Dialog.Title>New library</Dialog.Title>
			<Dialog.Description>
				The path must be a directory inside one of this server's configured library roots.
			</Dialog.Description>
		</Dialog.Header>
		<form class="flex flex-col gap-4" onsubmit={submit}>
			<div class="flex flex-col gap-2">
				<Label for="library-name">Name</Label>
				<Input id="library-name" bind:value={name} required placeholder="Comics" />
			</div>
			<div class="flex flex-col gap-2">
				<Label for="library-path">Path</Label>
				<Input id="library-path" bind:value={path} required placeholder="/data/comics" />
			</div>
			<div class="flex flex-col gap-2">
				<Label for="library-description">Description</Label>
				<Input id="library-description" bind:value={description} placeholder="Optional" />
			</div>
			<div class="grid gap-4 sm:grid-cols-2">
				<div class="flex flex-col gap-2">
					<Label for="library-type">Type</Label>
					<Select.Root
						type="single"
						value={libraryType}
						onValueChange={(value) => (libraryType = value as LibraryType)}
					>
						<Select.Trigger id="library-type" class="w-full">
							{LIBRARY_TYPE_LABELS[libraryType]}
						</Select.Trigger>
						<Select.Content>
							{#each Object.entries(LIBRARY_TYPE_LABELS) as [value, label] (value)}
								<Select.Item {value} {label} />
							{/each}
						</Select.Content>
					</Select.Root>
				</div>
				<div class="flex flex-col gap-2">
					<Label for="library-pattern">Layout</Label>
					<Select.Root
						type="single"
						value={libraryPattern}
						onValueChange={(value) => (libraryPattern = value as LibraryPattern)}
					>
						<Select.Trigger id="library-pattern" class="w-full">
							{LIBRARY_PATTERN_LABELS[libraryPattern]}
						</Select.Trigger>
						<Select.Content>
							{#each Object.entries(LIBRARY_PATTERN_LABELS) as [value, label] (value)}
								<Select.Item {value} {label} />
							{/each}
						</Select.Content>
					</Select.Root>
				</div>
			</div>
			<div class="flex items-center justify-between rounded-md border px-3 py-2">
				<Label for="library-watch" class="font-normal">Watch the folder for changes</Label>
				<Switch id="library-watch" bind:checked={watch} />
			</div>
			<div class="flex items-center justify-between rounded-md border px-3 py-2">
				<Label for="library-scan" class="font-normal">Scan right after creating</Label>
				<Switch id="library-scan" bind:checked={scanAfterPersist} />
			</div>
			<Dialog.Footer>
				<Button type="submit" disabled={!canSubmit || create.isPending}>
					{create.isPending ? 'Creating…' : 'Create library'}
				</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
