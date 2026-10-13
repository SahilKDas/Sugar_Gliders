function Get-WorkspaceBytes([string]$Root) {
    if (-not [IO.Directory]::Exists($Root)) { throw 'Workspace root is missing.' }
    [long]$total=0
    $pending=[Collections.Generic.Queue[string]]::new()
    $pending.Enqueue($Root)
    while ($pending.Count -gt 0) {
        $directory=$pending.Dequeue()
        try {
            foreach ($file in [IO.Directory]::GetFiles($directory)) {
                try { $total+=([IO.FileInfo]::new($file)).Length }
                catch [IO.FileNotFoundException] { }
                catch [IO.DirectoryNotFoundException] { }
            }
            foreach ($child in [IO.Directory]::GetDirectories($directory)) {
                try {
                    if (([IO.File]::GetAttributes($child) -band [IO.FileAttributes]::ReparsePoint) -eq 0) { $pending.Enqueue($child) }
                } catch [IO.DirectoryNotFoundException] { }
                catch [IO.FileNotFoundException] { }
            }
        } catch [IO.DirectoryNotFoundException] { }
        catch [IO.FileNotFoundException] { }
    }
    return $total
}
