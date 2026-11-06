// The bundle entry: runs in the page, reads the options the test put on `window`, installs the fake.
import { install, type Options } from './backend'

install((window as unknown as { __FAKE_OPTIONS__: Options }).__FAKE_OPTIONS__)
