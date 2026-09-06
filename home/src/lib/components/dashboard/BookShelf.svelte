<script lang="ts">
	import type { DashboardBookCardFragment } from '$lib/graphql/generated/graphql';
	import BookCard from './BookCard.svelte';
	import DashboardSection from './DashboardSection.svelte';

	let {
		title,
		description,
		books,
		pending = false,
		error = null,
		errorTitle,
		emptyTitle,
		emptyDescription,
		showProgress = false,
		now = new Date()
	}: {
		title: string;
		description?: string;
		books: readonly DashboardBookCardFragment[];
		pending?: boolean;
		error?: unknown;
		errorTitle: string;
		emptyTitle: string;
		emptyDescription?: string;
		showProgress?: boolean;
		now?: Date;
	} = $props();
</script>

<DashboardSection
	{title}
	{description}
	{pending}
	{error}
	{errorTitle}
	{emptyTitle}
	{emptyDescription}
	empty={books.length === 0}
	skeletonRows={1}
>
	<ul class="-mx-1 flex gap-4 overflow-x-auto px-1 pb-2">
		{#each books as book (book.id)}
			<li class="flex">
				<BookCard {book} {showProgress} {now} />
			</li>
		{/each}
	</ul>
</DashboardSection>
