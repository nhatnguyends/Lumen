import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

/**
 * Safe-mode build guard.
 *
 * Lumen must not ship or spawn PowerShell helpers. This check runs before the
 * production build so a future widget cannot quietly reintroduce one.
 */

export function checkSafe(root = process.cwd()) {
  const problems = []

  function walk(dir) {
    if (!fs.existsSync(dir)) return
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name)
      if (entry.isDirectory()) walk(full)
      else if (entry.isFile()) inspect(full)
    }
  }

  function inspect(file) {
    const rel = path.relative(root, file)
    if (/\.ps1$/i.test(file)) {
      problems.push(`${rel}: PowerShell script is not allowed in safe mode`)
      return
    }
    if (!/\.(ts|tsx|js|mjs|cjs|cs|xaml|py|pyw|rs|toml)$/i.test(file)) return
    const normalized = rel.replace(/\\/g, '/')
    const pythonHelper = normalized === 'tools/icon_extract.py' || normalized === 'tools/system_controls.py'
    const widgetPackage = normalized.startsWith('widgets/')
    const nativeDock = normalized.startsWith('native-dock/') || (widgetPackage && normalized.includes('/native/')) || normalized === 'startup_windows.py' || normalized.startsWith('scripts/') || normalized === 'run_native_dock.py' || normalized === 'run_native_clock.py' || normalized === 'run_lumen.py' || normalized === 'run_lumen.pyw' || normalized === 'Launch Lumen.pyw'
    if (!normalized.startsWith('src/') && !normalized.startsWith('wpf/Lumen/') && !pythonHelper && !nativeDock && !widgetPackage) return
    if (normalized.includes('/bin/') || normalized.includes('/obj/')) return
    const text = fs.readFileSync(file, 'utf8')
    if (/powershell\.exe/i.test(text)) {
      problems.push(`${rel}: launches powershell.exe`)
    }
    if (/-ExecutionPolicy/i.test(text)) {
      problems.push(`${rel}: uses -ExecutionPolicy`)
    }
    if (/CopyFromScreen|PrintWindow|SetWindowsHookEx|GetAsyncKeyState/i.test(text)) {
      problems.push(`${rel}: uses a capture/input-hook API`)
    }
    // All native packages and playback adapters prohibit synthetic input.
    if (
      (normalized.startsWith('src/') || normalized.startsWith('resources/') || pythonHelper || nativeDock || widgetPackage) &&
      /keybd_event|SendInput|mouse_event/i.test(text)
    ) {
      problems.push(`${rel}: injects synthetic input`)
    }
    if (nativeDock && /NetUserEnum|NetLocalGroupEnum|NetGroupEnum|Win32_UserAccount/i.test(text)) {
      problems.push(`${rel}: enumerates local accounts or groups`)
    }
  }

  walk(path.join(root, 'src', 'main'))
  walk(path.join(root, 'resources'))
  walk(path.join(root, 'wpf', 'Lumen'))
  inspect(path.join(root, 'tools', 'icon_extract.py'))
  if (fs.existsSync(path.join(root, 'tools', 'system_controls.py'))) inspect(path.join(root, 'tools', 'system_controls.py'))
  walk(path.join(root, 'native-dock', 'src'))
  walk(path.join(root, 'widgets'))
  if (fs.existsSync(path.join(root, 'native-dock', 'build.rs'))) inspect(path.join(root, 'native-dock', 'build.rs'))
  inspect(path.join(root, 'native-dock', 'Cargo.toml'))
  inspect(path.join(root, 'run_native_dock.py'))
  inspect(path.join(root, 'run_native_clock.py'))
  for(const name of ['run_lumen.py','run_lumen.pyw','Launch Lumen.pyw']) if(fs.existsSync(path.join(root,name))) inspect(path.join(root,name))
  if(fs.existsSync(path.join(root,'startup_windows.py'))) inspect(path.join(root,'startup_windows.py'))
  for(const name of ['build-portable.py','export-source.py','verify-portable.py','test-portable.py','third_party.py']) if(fs.existsSync(path.join(root,'scripts',name))) inspect(path.join(root,'scripts',name))
  walk(path.join(root,'scripts','portable_python'))
  return problems
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const problems = checkSafe()
  if (problems.length) {
    console.error('Safe-mode check failed:')
    for (const problem of problems) console.error(`  - ${problem}`)
    process.exit(1)
  }

  console.log('Safe-mode check: no PowerShell helpers found.')
}
