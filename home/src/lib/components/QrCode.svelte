<script lang="ts">
	import qrcode from 'qrcode-generator';

	let { value, label, size = 160 }: { value: string; label: string; size?: number } = $props();

	// Type 0 picks the smallest symbol that fits; M-level correction keeps
	// the module count low for phone cameras at arm's length.
	const svg = $derived.by(() => {
		const code = qrcode(0, 'M');
		code.addData(value);
		code.make();
		return code.createSvgTag({ cellSize: 4, margin: 2, scalable: true });
	});
</script>

<div
	class="rounded-md border bg-white p-1 [&>svg]:h-full [&>svg]:w-full"
	style="width: {size}px; height: {size}px"
	role="img"
	aria-label={label}
>
	{@html svg}
</div>
