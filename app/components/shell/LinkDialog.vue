<script setup lang="ts">
// A link from another app (ssh://user@host:22) asks Dalan to connect. It always asks first: Cancel has the focus, so a
// stray Enter never connects. Spec: replica/windows-installer/spec.md section 9.1.
const links = useLinksStore()
const open = computed({
  get: () => links.current !== null,
  set: (v: boolean) => { if (!v && links.current) links.cancel() },
})
</script>

<template>
  <AlertDialog v-model:open="open">
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>Open a connection to <span class="font-mono">{{ links.label }}</span>?</AlertDialogTitle>
        <AlertDialogDescription>
          <template v-if="links.savedHost">This is {{ links.savedHost.label }}, one of your saved servers. </template>
          A link in another app asked Dalan to connect. Check the address before you continue.
        </AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        <!-- Cancel closes the dialog, which drops the link (the setter above). Connect is a plain button: it does not
             close the dialog itself, so the two can never both act on the same link. -->
        <AlertDialogCancel data-autofocus>Cancel</AlertDialogCancel>
        <Button @click="links.connect()">Connect</Button>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
