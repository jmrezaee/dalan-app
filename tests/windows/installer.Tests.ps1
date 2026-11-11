# Pester tests for the Windows installer (replica/windows-installer/spec.md section 10, tests 1 to 6).
# They run in CI (job `windows-installer` in .github/workflows/ci.yml) and in a Windows VM:
#   $env:DALAN_SETUP_USER      the just-me setup  (Dalan_<ver>_x64-setup.exe)
#   $env:DALAN_SETUP_ALLUSERS  the all-users setup (Dalan_<ver>_x64-allusers-setup.exe)
#   $env:DALAN_PREV_SETUP      the previous release's just-me setup (optional: the upgrade test is skipped without it)
# The tests install and uninstall for real, so run them only on a throwaway machine or VM.

BeforeDiscovery {
  $script:HavePrev = [bool]($env:DALAN_PREV_SETUP -and (Test-Path $env:DALAN_PREV_SETUP))
}

BeforeAll {
  $script:SetupUser = $env:DALAN_SETUP_USER
  $script:SetupAll = $env:DALAN_SETUP_ALLUSERS
  $script:UserDir = Join-Path $env:LOCALAPPDATA 'Dalan'
  $script:AllDir = Join-Path $env:ProgramFiles 'Dalan'
  $script:Exe = 'dalan-app.exe'
  $script:UninstKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\Dalan'
  $script:StartMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Dalan.lnk'
  $script:CommonStartMenu = Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\Dalan.lnk'
  $script:Desktop = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Dalan.lnk'
  $script:DataDirs = @((Join-Path $env:APPDATA 'app.dalan.desktop'), (Join-Path $env:LOCALAPPDATA 'app.dalan.desktop'))
  $script:SshCommand = '"{0}" --open-url "%1"'

  # Runs a setup (or uninstaller) and returns its exit code.
  function Invoke-Setup([string]$Path, [string[]]$Arguments) {
    $p = Start-Process -FilePath $Path -ArgumentList $Arguments -Wait -PassThru
    return $p.ExitCode
  }
  # The uninstaller copies itself to %TEMP% and returns at once, unless it is told to run in place (_?=).
  function Invoke-Uninstall([string]$Dir, [string[]]$Arguments = @('/S')) {
    return Invoke-Setup (Join-Path $Dir 'uninstall.exe') ($Arguments + "_?=$Dir")
  }
  # Where a shortcut points. (The template re-targets existing shortcuts on an update, so their timestamps change.)
  function Get-LinkTarget([string]$Path) { (New-Object -ComObject WScript.Shell).CreateShortcut($Path).TargetPath }
  function Get-Reg($Hive, $Key, $Name) { (Get-ItemProperty -Path "${Hive}:\$Key" -Name $Name -ErrorAction SilentlyContinue).$Name }
  # Back to a clean PC: uninstall whatever is there, then remove what is left.
  function Reset-Dalan {
    foreach ($dir in @($script:UserDir, $script:AllDir)) {
      if (Test-Path (Join-Path $dir 'uninstall.exe')) { Invoke-Uninstall $dir @('/S', '/DELETEDATA=1') | Out-Null }
      Remove-Item $dir -Recurse -Force -ErrorAction SilentlyContinue
    }
    foreach ($dir in $script:DataDirs) { Remove-Item $dir -Recurse -Force -ErrorAction SilentlyContinue }
    foreach ($f in @($script:StartMenu, $script:CommonStartMenu, $script:Desktop)) { Remove-Item $f -Force -ErrorAction SilentlyContinue }
    foreach ($k in @('HKCU:\Software\Classes\ssh', 'HKCU:\Software\Classes\Dalan.ssh', 'HKCU:\Software\Dalan', 'HKLM:\Software\Dalan', "HKCU:\$($script:UninstKey)", "HKLM:\$($script:UninstKey)")) {
      Remove-Item $k -Recurse -Force -ErrorAction SilentlyContinue
    }
    Remove-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'Dalan' -ErrorAction SilentlyContinue
    Remove-ItemProperty 'HKCU:\Software\RegisteredApplications' -Name 'Dalan' -ErrorAction SilentlyContinue
  }
}

Describe 'Dalan installer' {
  BeforeEach { Reset-Dalan }
  AfterAll { Reset-Dalan }

  It '1. a silent just-me install needs no administrator and writes the choices' {
    # The manifest says "asInvoker": the setup never asks for administrator rights, and nothing lands in HKLM.
    [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($SetupUser)) | Should -Match 'asInvoker'

    Invoke-Setup $SetupUser @('/S', '/DESKTOP=1', '/SSHLINKS=1', '/AUTOSTART=1') | Should -Be 0

    Join-Path $UserDir $Exe | Should -Exist
    $StartMenu | Should -Exist
    $Desktop | Should -Exist
    (Get-Reg HKCU 'Software\Classes\ssh\shell\open\command' '(default)') | Should -Be ($SshCommand -f (Join-Path $UserDir $Exe))
    (Get-Item 'HKCU:\Software\Classes\ssh').GetValueNames() | Should -Contain 'URL Protocol'
    (Get-Reg HKCU 'Software\RegisteredApplications' 'Dalan') | Should -Be 'Software\Dalan\Capabilities'
    (Get-Reg HKCU 'Software\Dalan\Capabilities\URLAssociations' 'ssh') | Should -Be 'Dalan.ssh'
    (Get-Reg HKCU 'Software\Microsoft\Windows\CurrentVersion\Run' 'Dalan') | Should -Match '--minimized$'
    (Get-Reg HKCU $UninstKey 'DisplayName') | Should -Be 'Dalan'
    (Get-Reg HKCU $UninstKey 'Publisher') | Should -Not -BeNullOrEmpty
    (Get-Reg HKCU $UninstKey 'DisplayVersion') | Should -Not -BeNullOrEmpty
    $k = 'Software\Dalan\Installer'
    (Get-Reg HKCU $k 'StartMenu') | Should -Be 1
    (Get-Reg HKCU $k 'Desktop') | Should -Be 1
    (Get-Reg HKCU $k 'SshLinks') | Should -Be 1
    (Get-Reg HKCU $k 'Autostart') | Should -Be 1
    (Get-Reg HKCU $k 'Edition') | Should -Be 'user'
    Test-Path "HKLM:\$UninstKey" | Should -BeFalse
  }

  It '1b. the silent defaults are a Start menu shortcut and nothing else' {
    Invoke-Setup $SetupUser @('/S') | Should -Be 0
    $StartMenu | Should -Exist
    $Desktop | Should -Not -Exist
    Test-Path 'HKCU:\Software\Classes\ssh' | Should -BeFalse
    (Get-Reg HKCU 'Software\Microsoft\Windows\CurrentVersion\Run' 'Dalan') | Should -BeNullOrEmpty
  }

  It '2. an update keeps the choices, the links, the shortcuts and the data' -Skip:(-not $HavePrev) {
    Invoke-Setup $env:DALAN_PREV_SETUP @('/S') | Should -Be 0
    New-Item -ItemType Directory -Force $DataDirs[0] | Out-Null
    Set-Content (Join-Path $DataDirs[0] 'marker.txt') 'vault'

    Invoke-Setup $SetupUser @('/S', '/DESKTOP=1', '/SSHLINKS=1', '/AUTOSTART=1') | Should -Be 0
    $before = @($StartMenu, $Desktop | ForEach-Object { Get-LinkTarget $_ })

    # What the auto-updater runs
    Invoke-Setup $SetupUser @('/S', '/UPDATE') | Should -Be 0

    $k = 'Software\Dalan\Installer'
    (Get-Reg HKCU $k 'SshLinks') | Should -Be 1
    (Get-Reg HKCU $k 'Autostart') | Should -Be 1
    (Get-Reg HKCU 'Software\Classes\ssh\shell\open\command' '(default)') | Should -Be ($SshCommand -f (Join-Path $UserDir $Exe))
    (Get-Reg HKCU 'Software\Microsoft\Windows\CurrentVersion\Run' 'Dalan') | Should -Match '--minimized$'
    @($StartMenu, $Desktop | ForEach-Object { Get-LinkTarget $_ }) | Should -Be $before
    Join-Path $DataDirs[0] 'marker.txt' | Should -Exist
  }

  It '3. uninstalling keeps the data and a handler that is not ours' {
    # Another app's handler, set before the install; the install does not touch it (SSHLINKS=0)
    New-Item -Path 'HKCU:\Software\Classes\ssh\shell\open\command' -Force | Out-Null
    Set-ItemProperty 'HKCU:\Software\Classes\ssh\shell\open\command' -Name '(default)' -Value '"C:\Tools\other.exe" "%1"'
    Invoke-Setup $SetupUser @('/S', '/SSHLINKS=0', '/AUTOSTART=1') | Should -Be 0
    New-Item -ItemType Directory -Force $DataDirs[0] | Out-Null
    Set-Content (Join-Path $DataDirs[0] 'marker.txt') 'vault'

    Invoke-Uninstall $UserDir | Should -Be 0

    Join-Path $UserDir $Exe | Should -Not -Exist
    $StartMenu | Should -Not -Exist
    Test-Path "HKCU:\$UninstKey" | Should -BeFalse
    (Get-Reg HKCU 'Software\Microsoft\Windows\CurrentVersion\Run' 'Dalan') | Should -BeNullOrEmpty
    Join-Path $DataDirs[0] 'marker.txt' | Should -Exist
    (Get-Reg HKCU 'Software\Classes\ssh\shell\open\command' '(default)') | Should -Be '"C:\Tools\other.exe" "%1"'
  }

  It '3b. uninstalling removes our own ssh:// links' {
    Invoke-Setup $SetupUser @('/S', '/SSHLINKS=1') | Should -Be 0
    Invoke-Uninstall $UserDir | Should -Be 0
    Test-Path 'HKCU:\Software\Classes\ssh' | Should -BeFalse
    Test-Path 'HKCU:\Software\Classes\Dalan.ssh' | Should -BeFalse
    (Get-Reg HKCU 'Software\RegisteredApplications' 'Dalan') | Should -BeNullOrEmpty
  }

  It '4. uninstalling with /DELETEDATA=1 deletes both data folders' {
    Invoke-Setup $SetupUser @('/S') | Should -Be 0
    foreach ($d in $DataDirs) { New-Item -ItemType Directory -Force $d | Out-Null; Set-Content (Join-Path $d 'marker.txt') 'x' }

    Invoke-Uninstall $UserDir @('/S', '/DELETEDATA=1') | Should -Be 0

    foreach ($d in $DataDirs) { $d | Should -Not -Exist }
    Test-Path 'HKCU:\Software\Dalan\Installer' | Should -BeFalse
  }

  It '5. the all-users edition installs in Program Files, HKLM and the common Start menu' {
    Invoke-Setup $SetupAll @('/S') | Should -Be 0
    Join-Path $AllDir $Exe | Should -Exist
    $CommonStartMenu | Should -Exist
    (Get-Reg HKLM $UninstKey 'DisplayName') | Should -Be 'Dalan'
    (Get-Reg HKLM 'Software\Dalan\Installer' 'Edition') | Should -Be 'allusers'
  }

  It '6. the other edition refuses to install on top of it' {
    Invoke-Setup $SetupAll @('/S') | Should -Be 0
    Invoke-Setup $SetupUser @('/S') | Should -Not -Be 0
    Join-Path $UserDir $Exe | Should -Not -Exist
  }

  # Only when the build is signed (the signing secrets exist): DALAN_EXPECT_SIGNED=1.
  It '7. both setups and the installed uninstaller carry a valid signature' -Skip:($env:DALAN_EXPECT_SIGNED -ne '1') {
    foreach ($f in @($SetupUser, $SetupAll)) { (Get-AuthenticodeSignature $f).Status | Should -Be 'Valid' }
    Invoke-Setup $SetupUser @('/S') | Should -Be 0
    (Get-AuthenticodeSignature (Join-Path $UserDir 'uninstall.exe')).Status | Should -Be 'Valid'
  }

  # The behaviour of a link is covered by the Rust and browser tests; here the installed app must simply start with
  # one (and with injection attempts) and keep running, and nothing a link says may start another program.
  It '8. the installed app starts with a valid link and with injection links' {
    Invoke-Setup $SetupUser @('/S') | Should -Be 0
    $app = Join-Path $UserDir $Exe
    foreach ($link in @('ssh://deploy@10.0.0.5:22', 'ssh://-oProxyCommand=calc@h', 'ssh://h -o x', 'ssh://%2Doops@h')) {
      $p = Start-Process -FilePath $app -ArgumentList @('--open-url', "`"$link`"") -PassThru
      try {
        Start-Sleep -Seconds 8
        $p.HasExited | Should -BeFalse -Because "the app should still be running after $link"
      }
      finally {
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 1
      }
    }
    Get-Process -Name 'calc', 'CalculatorApp' -ErrorAction SilentlyContinue | Should -BeNullOrEmpty
  }
}
