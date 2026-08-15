# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_cfbench_global_optspecs
    string join \n ipv4 ipv6 json no-download no-upload no-loaded-latency no-metadata timeout= q/quiet verbose rpki-check h/help V/version
end

function __fish_cfbench_needs_command
    # Figure out if the current invocation already has a command.
    set -l cmd (commandline -opc)
    set -e cmd[1]
    argparse -s (__fish_cfbench_global_optspecs) -- $cmd 2>/dev/null
    or return
    if set -q argv[1]
        # Also print the command, so this can be used to figure out what it is.
        echo $argv[1]
        return 1
    end
    return 0
end

function __fish_cfbench_using_subcommand
    set -l cmd (__fish_cfbench_needs_command)
    test -z "$cmd"
    and return 1
    contains -- $cmd[1] $argv
end

complete -c cfbench -n "__fish_cfbench_needs_command" -l timeout -d 'Per-request timeout' -r
complete -c cfbench -n "__fish_cfbench_needs_command" -l ipv4 -d 'Use IPv4 only'
complete -c cfbench -n "__fish_cfbench_needs_command" -l ipv6 -d 'Use IPv6 only'
complete -c cfbench -n "__fish_cfbench_needs_command" -l json -d 'Emit versioned JSON to stdout'
complete -c cfbench -n "__fish_cfbench_needs_command" -l no-download -d 'Skip download measurements'
complete -c cfbench -n "__fish_cfbench_needs_command" -l no-upload -d 'Skip upload measurements'
complete -c cfbench -n "__fish_cfbench_needs_command" -l no-loaded-latency -d 'Disable latency probes during transfers'
complete -c cfbench -n "__fish_cfbench_needs_command" -l no-metadata -d 'Skip the default public IP and network metadata request'
complete -c cfbench -n "__fish_cfbench_needs_command" -s q -l quiet -d 'Suppress normal output; report status with the exit code'
complete -c cfbench -n "__fish_cfbench_needs_command" -l verbose -d 'Show per-request measurement progress'
complete -c cfbench -n "__fish_cfbench_needs_command" -l rpki-check -d 'Perform an informational reachability probe to Cloudflare\'s RPKI-invalid route'
complete -c cfbench -n "__fish_cfbench_needs_command" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c cfbench -n "__fish_cfbench_needs_command" -s V -l version -d 'Print version'
complete -c cfbench -n "__fish_cfbench_needs_command" -f -a "completions" -d 'Generate a shell completion script'
complete -c cfbench -n "__fish_cfbench_needs_command" -f -a "man" -d 'Generate the cfbench(1) manual page'
complete -c cfbench -n "__fish_cfbench_needs_command" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c cfbench -n "__fish_cfbench_using_subcommand completions" -s h -l help -d 'Print help'
complete -c cfbench -n "__fish_cfbench_using_subcommand man" -s h -l help -d 'Print help'
complete -c cfbench -n "__fish_cfbench_using_subcommand help; and not __fish_seen_subcommand_from completions man help" -f -a "completions" -d 'Generate a shell completion script'
complete -c cfbench -n "__fish_cfbench_using_subcommand help; and not __fish_seen_subcommand_from completions man help" -f -a "man" -d 'Generate the cfbench(1) manual page'
complete -c cfbench -n "__fish_cfbench_using_subcommand help; and not __fish_seen_subcommand_from completions man help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
