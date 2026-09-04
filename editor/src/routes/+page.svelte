<script lang="ts">
	import { resolve } from '$app/paths';
	import { Button } from '$lib/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '$lib/components/ui/card';
</script>

<svelte:head>
	<title>Stump · Ingest editor</title>
</svelte:head>

<div class="mx-auto flex min-h-[60vh] max-w-2xl items-center justify-center">
	<Card class="w-full">
		<CardHeader>
			<CardTitle>Staged ingest, under control</CardTitle>
			<CardDescription>
				Upload books into an immutable staging area, inspect quality evidence, and approve metadata before
				commit.
			</CardDescription>
		</CardHeader>
		<CardContent class="flex flex-wrap gap-3">
			<Button href={resolve('/drop')}>Open drop folder</Button>
			<Button href={resolve('/queue')} variant="outline">View analysis queue</Button>
		</CardContent>
	</Card>
</div>
