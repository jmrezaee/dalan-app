<script setup lang="ts">
import { Folder, Key, Network, Plus, ScrollText, Server, ShieldCheck, Terminal, Trash2, Zap } from '@lucide/vue'
import { toast } from 'vue-sonner'

const nav = [
  { id: 'hosts', label: 'Hosts', icon: Server, count: 12 },
  { id: 'keychain', label: 'Keychain', icon: Key, count: 3 },
  { id: 'pf', label: 'Port forwarding', icon: Network },
  { id: 'snippets', label: 'Snippets', icon: Zap },
  { id: 'known', label: 'Known hosts', icon: ShieldCheck },
  { id: 'logs', label: 'Logs', icon: ScrollText },
]
const activeNav = ref('hosts')
const q = ref('')
const sw = ref(true)
const radio = ref('local')
const tab = ref('snippets')
const confirmOpen = ref(false)
const overlay = ref<'connecting' | 'host-key-new' | 'host-key-changed' | 'auth-failed' | 'unreachable' | 'disconnected'>('host-key-new')
const overlays = ['connecting', 'host-key-new', 'host-key-changed', 'auth-failed', 'unreachable', 'disconnected'] as const
const fp = 'SHA256:3b0c2f4d9a1e8c7f5b6a0d2e4c8f1a3b5d7e9f0a2c4'
</script>

<template>
  <div class="flex flex-col gap-6 text-sm">
    <!-- Buttons -->
    <Card title="Button">
      <div class="flex flex-wrap items-center gap-2">
        <Button>Trust and connect</Button><Button variant="secondary">Cancel</Button><Button variant="ghost">Skip</Button>
        <Button variant="destructive">Delete 3 hosts</Button><Button variant="link">Change</Button>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <Button class="bg-accent-hover">Hover</Button><Button class="outline-2 outline-offset-2 outline-accent">Focus</Button>
        <Button loading>Connecting…</Button><Button disabled>Disabled</Button>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <Button size="xs" variant="secondary">xs</Button><Button size="sm" variant="secondary">sm</Button><Button variant="secondary">md</Button><Button size="lg">Unlock</Button>
        <IconButton label="Delete"><Trash2 /></IconButton><IconButton label="Pinned" pressed><Folder /></IconButton>
        <SplitButton label="New host"><template #icon><Plus /></template><DropdownMenuItem>Host</DropdownMenuItem><DropdownMenuItem>Group</DropdownMenuItem><DropdownMenuItem>Import</DropdownMenuItem></SplitButton>
      </div>
    </Card>

    <!-- Inputs -->
    <Card title="Input">
      <div class="grid grid-cols-2 gap-3.5">
        <div class="flex flex-col gap-1.5"><Label for="i1">Default</Label><Input id="i1" placeholder="10.0.0.5" /></div>
        <div class="flex flex-col gap-1.5"><Label for="i2">Focus</Label><Input id="i2" model-value="db.internal" class="border-accent ring-2 ring-accent/35" /></div>
        <div class="flex flex-col gap-1.5"><Label for="i3">Inherited</Label><Input id="i3" placeholder="deploy (from Production)" class="italic" /></div>
        <div class="flex flex-col gap-1.5"><Label for="i4">Invalid</Label><NumberInput id="i4" model-value="70000" invalid /><span class="text-xs text-danger">Use a port between 1 and 65535.</span></div>
        <div class="flex flex-col gap-1.5"><Label for="i5">Password</Label><PasswordInput id="i5" /></div>
        <div class="flex flex-col gap-1.5"><Label for="i6">Saved secret</Label><PasswordInput id="i6" saved /></div>
        <div class="flex flex-col gap-1.5"><Label for="i7">Disabled</Label><Input id="i7" model-value="22" disabled /></div>
        <div class="flex flex-col gap-1.5"><Label>Search</Label><SearchInput v-model="q" /></div>
        <div class="col-span-2 flex flex-col gap-1.5"><Label for="i8">Notes</Label><Textarea id="i8" placeholder="Anything worth remembering" /></div>
        <div class="flex flex-col gap-1.5">
          <Label>Select</Label>
          <Select default-value="a"><SelectTrigger class="w-full"><SelectValue /></SelectTrigger>
            <SelectContent><SelectItem value="a">Password</SelectItem><SelectItem value="b">deploy-ed25519</SelectItem></SelectContent></Select>
        </div>
      </div>
    </Card>

    <Card title="Selection controls">
      <div class="flex flex-wrap items-center gap-6">
        <label class="flex items-center gap-2"><Switch v-model="sw" />Agent forwarding</label>
        <label class="flex items-center gap-2"><Switch :model-value="false" />Save passphrase</label>
        <label class="flex items-center gap-2"><Checkbox :model-value="true" />web-1</label>
        <label class="flex items-center gap-2"><Checkbox :model-value="false" />web-2</label>
        <RadioGroup v-model="radio" class="flex gap-4">
          <label class="flex items-center gap-2"><RadioGroupItem value="local" />Local</label>
          <label class="flex items-center gap-2"><RadioGroupItem value="remote" />Remote</label>
        </RadioGroup>
      </div>
      <Tabs v-model="tab">
        <TabsList class="h-8 gap-5 rounded-none border-b border-border bg-transparent p-0">
          <TabsTrigger v-for="t in ['snippets', 'history', 'theme']" :key="t" :value="t" class="h-8 rounded-none border-0 border-b-2 border-transparent bg-transparent px-0 capitalize text-muted shadow-none data-[state=active]:border-accent data-[state=active]:bg-transparent data-[state=active]:text-text data-[state=active]:shadow-none">{{ t }}</TabsTrigger>
        </TabsList>
      </Tabs>
    </Card>

    <Card title="Badges, tags, kbd, status">
      <div class="flex flex-wrap items-center gap-2">
        <Badge>ED25519</Badge><Badge variant="success">Trusted</Badge><Badge variant="warning">Changed</Badge><Badge variant="destructive">Failed</Badge>
        <TagChip label="prod" removable /><TagChip label="web" />
        <Kbd>⌘K</Kbd><Kbd>⌘T</Kbd>
      </div>
      <div class="flex flex-wrap items-center gap-4">
        <span class="flex items-center gap-1.5"><StatusDot status="unread" label="web-1: new output" />unread</span>
        <span class="flex items-center gap-1.5"><StatusDot status="waiting" label="web-1: waiting for input" />needs input</span>
        <span class="flex items-center gap-1.5"><StatusDot status="error" label="web-1: disconnected" />error</span>
        <StatusPill tone="success">Active · 2</StatusPill><StatusPill tone="warning">Reconnecting</StatusPill><StatusPill tone="danger">Error</StatusPill><StatusPill tone="neutral">Stopped</StatusPill>
      </div>
    </Card>

    <!-- Floating layers: triggers -->
    <Card title="Dialogs, sheet, menus, toast, palette">
      <div class="flex flex-wrap items-center gap-2">
        <Dialog>
          <DialogTrigger as-child><Button variant="secondary">Dialog</Button></DialogTrigger>
          <DialogContent class="sm:max-w-[400px]">
            <DialogHeader><DialogTitle>Rename host</DialogTitle><DialogDescription>Shown in your hosts list and on tabs.</DialogDescription></DialogHeader>
            <Input model-value="web-1" aria-label="Host name" />
            <DialogFooter><Button variant="secondary">Cancel</Button><Button>Save</Button></DialogFooter>
          </DialogContent>
        </Dialog>
        <AlertDialog v-model:open="confirmOpen">
          <AlertDialogTrigger as-child><Button variant="secondary">Alert dialog</Button></AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader><AlertDialogTitle>Delete 3 hosts?</AlertDialogTitle><AlertDialogDescription>Their saved passwords and history go with them. This can't be undone.</AlertDialogDescription></AlertDialogHeader>
            <AlertDialogFooter><AlertDialogCancel>Cancel</AlertDialogCancel><AlertDialogAction class="bg-danger-solid text-on-danger">Delete 3 hosts</AlertDialogAction></AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
        <Sheet>
          <SheetTrigger as-child><Button variant="secondary">Sheet</Button></SheetTrigger>
          <SheetContent class="w-[440px] sm:max-w-[440px]">
            <SheetHeader><SheetTitle>Edit host</SheetTitle><SheetDescription>Empty fields inherit from the group.</SheetDescription></SheetHeader>
            <div class="flex flex-col gap-1.5 px-4"><Label for="sh1">Address</Label><Input id="sh1" placeholder="10.0.0.5" /></div>
            <SheetFooter><Button variant="secondary">Cancel</Button><Button>Save</Button></SheetFooter>
          </SheetContent>
        </Sheet>
        <DropdownMenu>
          <DropdownMenuTrigger as-child><Button variant="secondary">Menu</Button></DropdownMenuTrigger>
          <DropdownMenuContent class="w-56">
            <DropdownMenuItem>Connect<DropdownMenuShortcut>↵</DropdownMenuShortcut></DropdownMenuItem>
            <DropdownMenuItem>Open SFTP</DropdownMenuItem><DropdownMenuItem>Edit<DropdownMenuShortcut>⌘E</DropdownMenuShortcut></DropdownMenuItem>
            <DropdownMenuSeparator /><DropdownMenuItem variant="destructive">Delete</DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <ContextMenu>
          <ContextMenuTrigger class="rounded-md border border-dashed border-border-input px-3 py-1.5 text-muted">Right-click me</ContextMenuTrigger>
          <ContextMenuContent class="w-48"><ContextMenuItem>Connect</ContextMenuItem><ContextMenuItem>Duplicate</ContextMenuItem><ContextMenuItem variant="destructive">Delete</ContextMenuItem></ContextMenuContent>
        </ContextMenu>
        <Button variant="secondary" @click="toast.success('Saved.')">Toast ok</Button>
        <Button variant="secondary" @click="toast.error('Couldn\'t copy notes.txt: permission denied.')">Toast error</Button>
        <Button variant="secondary" @click="toast('Public key copied.')">Toast info</Button>
      </div>
      <Command class="max-w-[640px] rounded-lg border border-border bg-surface-raised shadow-pop">
        <CommandInput placeholder="Search hosts, tabs, snippets…" />
        <CommandList>
          <CommandEmpty>No matches. Press ⌘N to add a host</CommandEmpty>
          <CommandGroup heading="Hosts"><CommandItem value="web-1"><Terminal />web-1<CommandShortcut>deploy@10.0.0.5</CommandShortcut></CommandItem><CommandItem value="db-1"><Terminal />db-1<CommandShortcut>admin@10.0.0.9</CommandShortcut></CommandItem></CommandGroup>
          <CommandGroup heading="Actions"><CommandItem value="new-host"><Plus />New host<CommandShortcut>⌘N</CommandShortcut></CommandItem></CommandGroup>
        </CommandList>
      </Command>
    </Card>

    <Card title="Empty state, skeleton">
      <EmptyState :icon="Server" title="No servers yet" text="Add one by hand, or bring them over from your SSH config.">
        <Button>Add host</Button><Button variant="secondary">Import</Button>
      </EmptyState>
      <div class="flex flex-col gap-2"><Skeleton class="h-14 w-full bg-surface-hover" /><Skeleton class="h-14 w-full bg-surface-hover" /></div>
    </Card>

    <!-- App components -->
    <Card title="TabBar and VaultNav">
      <TabBar>
        <AppTab label="Vault" active /><AppTab label="SFTP" />
        <AppTab label="web-1" closable><template #leading><StatusDot status="unread" label="web-1: new output" /></template></AppTab>
        <AppTab label="db-1" closable><template #leading><StatusDot status="waiting" label="db-1: waiting for input" /></template></AppTab>
        <AppTab label="app-3" closable><template #leading><StatusDot status="error" label="app-3: disconnected" /></template></AppTab>
        <AppTab label="Release (3)" unsaved drop-target />
      </TabBar>
      <div class="flex gap-4">
        <VaultNav :items="nav" :active="activeNav" @select="activeNav = $event" />
        <VaultNav :items="nav" :active="activeNav" collapsed @select="activeNav = $event" />
      </div>
      <PageToolbar title="Hosts"><SearchInput v-model="q" /><SplitButton label="New host"><DropdownMenuItem>Host</DropdownMenuItem></SplitButton></PageToolbar>
    </Card>

    <Card title="Host, group, key and rule cards">
      <div class="grid grid-cols-2 gap-3">
        <HostCard label="web-1" user="deploy" address="10.0.0.5" os-letter="U" :tags="['prod', 'web']" />
        <HostCard label="Hover/selected" user="deploy" address="10.0.0.9" os-letter="D" selected />
        <HostCard label="Connecting" user="deploy" address="10.0.0.6" connecting />
        <HostCard label="Connected" user="admin" address="db.internal" :port="2222" connected multi-select checked />
        <GroupCard label="Production" :hosts="12" :groups="2" /><GroupCard label="Drop target" :hosts="1" drop-target />
      </div>
      <KeyCard label="deploy-ed25519" type="ED25519" fingerprint="SHA256:3b0c2f4d9a1e8c7f5b6a0d2e4c8f1a3b5d7e9f0a2c4" passphrase-saved />
      <div class="flex flex-col gap-2">
        <RuleCard kind="L" label="postgres" route="localhost:5432 → db.internal:5432 via bastion" state="active" :connections="2" />
        <RuleCard kind="R" label="webhook" route="remote:8080 → localhost:3000" state="stopped" />
        <RuleCard kind="D" label="socks" route="localhost:1080" state="reconnecting" />
        <RuleCard kind="L" label="postgres 2" route="localhost:5432 → db.internal:5432" state="error" error="Port 5432 is already in use on this computer. Pick another, or stop whatever is using it." />
      </div>
    </Card>

    <Card title="Progress rows, pane headers, stepper">
      <ProgressRow name="release-2.4.tar.gz" direction="up" :percent="64" speed="42 MB/s" eta="8 s left" />
      <ProgressRow name="notes.txt" direction="down" :percent="100" status="done" />
      <ProgressRow name="big.iso" direction="down" :percent="30" status="failed" />
      <PaneHeader title="web-1" status="unread" status-label="web-1: new output" focused broadcasting />
      <PaneHeader title="db-1" status="idle" status-label="db-1: idle" />
      <Stepper :steps="['Type', 'Where', 'Review']" :current="1" />
    </Card>

    <Card title="Status overlay (S18)">
      <div class="flex flex-wrap gap-1.5"><Button v-for="o in overlays" :key="o" size="xs" :variant="overlay === o ? 'default' : 'secondary'" @click="overlay = o">{{ o }}</Button></div>
      <div class="relative h-[380px] overflow-hidden rounded-md bg-terminal-bg p-3 font-mono text-[13px] text-terminal-fg">
        <TerminalSample />
        <StatusOverlay
          :kind="overlay" host="web-1" address="10.0.0.5:22" hop="bastion" :fingerprint="fp"
          old-fingerprint="SHA256:9f8e7d6c5b4a39281706f5e4d3c2b1a09f8e7d6c5b4" reason="web-1 rejected the password." time="14:32"
        />
      </div>
    </Card>

    <Card title="Unlock (S24)">
      <div class="flex flex-wrap justify-center gap-6">
        <UnlockCard mode="create" :strength="4" password="correct-horse-staple" confirm="correct-horse-staple" />
        <UnlockCard mode="unlock" error="That password didn't work." caps-lock />
      </div>
    </Card>

    <Card title="Terminal palette">
      <div class="rounded-md bg-terminal-bg p-3 font-mono text-[13px] text-terminal-fg"><TerminalSample /></div>
    </Card>
  </div>
</template>
