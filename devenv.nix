{ pkgs, config, ... }:

{
  # Rust template: stable toolchain supplied through rust-overlay.
  languages.rust = {
    enable = true;
    channel = "stable";
  };

  # Amaranth and python-electronics templates share one locked uv environment.
  languages.python = {
    enable = true;
    package = pkgs.python312;
    venv.enable = true;
    uv = {
      enable = true;
      sync = {
        enable = true;
        arguments = [ "--locked" ];
      };
    };
  };

  packages = with pkgs; [
    # Amaranth template, plus iCE40 synthesis/place-route/packing/programming.
    yosys
    surfer
    nextpnr
    icestorm
    # Formal template and RTL induction/cover tools (sby is SymbiYosys).
    tlaplus
    alloy6
    sby
    z3
  ];

  # Keep devenv's Python sitecustomize path while making hardware importable.
  enterShell = ''
    export PYTHONPATH="${config.devenv.root}/hw''${PYTHONPATH:+:$PYTHONPATH}"
  '';

  scripts.mov-assemble.exec = ''
    exec cargo run --manifest-path "${config.devenv.root}/Cargo.toml" -- assemble "$@"
  '';
  scripts.mov-run.exec = ''
    exec cargo run --manifest-path "${config.devenv.root}/Cargo.toml" -- run "$@"
  '';
  scripts.mov-formal.exec = ''
    exec python "${config.devenv.root}/formal/run.py" "$@"
  '';
  scripts.mov-check.exec = ''
    exec python "${config.devenv.root}/scripts/verify.py" "$@"
  '';
  scripts.mov-build.exec = ''
    exec python -m mov_machine.build "$@"
  '';
}
