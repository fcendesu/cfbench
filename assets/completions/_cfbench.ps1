
using namespace System.Management.Automation
using namespace System.Management.Automation.Language

Register-ArgumentCompleter -Native -CommandName 'cfbench' -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $commandElements = $commandAst.CommandElements
    $command = @(
        'cfbench'
        for ($i = 1; $i -lt $commandElements.Count; $i++) {
            $element = $commandElements[$i]
            if ($element -isnot [StringConstantExpressionAst] -or
                $element.StringConstantType -ne [StringConstantType]::BareWord -or
                $element.Value.StartsWith('-') -or
                $element.Value -eq $wordToComplete) {
                break
        }
        $element.Value
    }) -join ';'

    $completions = @(switch ($command) {
        'cfbench' {
            [CompletionResult]::new('--timeout', '--timeout', [CompletionResultType]::ParameterName, 'Per-request timeout')
            [CompletionResult]::new('--ipv4', '--ipv4', [CompletionResultType]::ParameterName, 'Use IPv4 only')
            [CompletionResult]::new('--ipv6', '--ipv6', [CompletionResultType]::ParameterName, 'Use IPv6 only')
            [CompletionResult]::new('--json', '--json', [CompletionResultType]::ParameterName, 'Emit versioned JSON to stdout')
            [CompletionResult]::new('--no-download', '--no-download', [CompletionResultType]::ParameterName, 'Skip download measurements')
            [CompletionResult]::new('--no-upload', '--no-upload', [CompletionResultType]::ParameterName, 'Skip upload measurements')
            [CompletionResult]::new('--no-loaded-latency', '--no-loaded-latency', [CompletionResultType]::ParameterName, 'Disable latency probes during transfers')
            [CompletionResult]::new('--no-metadata', '--no-metadata', [CompletionResultType]::ParameterName, 'Skip the default public IP and network metadata request')
            [CompletionResult]::new('-q', '-q', [CompletionResultType]::ParameterName, 'Suppress normal output; report status with the exit code')
            [CompletionResult]::new('--quiet', '--quiet', [CompletionResultType]::ParameterName, 'Suppress normal output; report status with the exit code')
            [CompletionResult]::new('--verbose', '--verbose', [CompletionResultType]::ParameterName, 'Show per-request measurement progress')
            [CompletionResult]::new('--rpki-check', '--rpki-check', [CompletionResultType]::ParameterName, 'Perform an informational reachability probe to Cloudflare''s RPKI-invalid route')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('-V', '-V ', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('--version', '--version', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('completions', 'completions', [CompletionResultType]::ParameterValue, 'Generate a shell completion script')
            [CompletionResult]::new('man', 'man', [CompletionResultType]::ParameterValue, 'Generate the cfbench(1) manual page')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'cfbench;completions' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'cfbench;man' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'cfbench;help' {
            [CompletionResult]::new('completions', 'completions', [CompletionResultType]::ParameterValue, 'Generate a shell completion script')
            [CompletionResult]::new('man', 'man', [CompletionResultType]::ParameterValue, 'Generate the cfbench(1) manual page')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'cfbench;help;completions' {
            break
        }
        'cfbench;help;man' {
            break
        }
        'cfbench;help;help' {
            break
        }
    })

    $completions.Where{ $_.CompletionText -like "$wordToComplete*" } |
        Sort-Object -Property ListItemText
}
