{
  description = "mrtklib-egui: an RTKLIB-style GUI suite for MRTKLIB (optional Nix support)";

  # flake.lock pins nixpkgs; move it with `nix flake update`.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      apps = [
        "navi"
        "post"
        "conv"
        "plot"
      ];

      # winit, glutin, wgpu and rfd load these at run time; they are not on the
      # library path of a Nix-built program.
      runtimeLibraries = pkgs: [
        pkgs.dbus.lib
        pkgs.wayland
        pkgs.libxkbcommon
        pkgs.libGL
        pkgs.vulkan-loader
        pkgs.libx11
        pkgs.libxcursor
        pkgs.libxi
        pkgs.libxrandr
      ];
    in
    {
      packages = forSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "mrtklib-egui";
          version = "0.0.0";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = map (app: "--package=mrtklib-egui-${app}") apps;
          # The tests open local sockets and start processes; run them with
          # `cargo test` in the development shell.
          doCheck = false;
          # The libraries go into the executables' run path instead of a
          # wrapper script: a wrapper renames the executable, and navi derives
          # its configuration filename from the executable name.
          # addDriverRunpath adds /run/opengl-driver/lib, the GPU driver on NixOS.
          nativeBuildInputs = [ pkgs.addDriverRunpath ];
          postFixup = pkgs.lib.concatMapStrings (app: ''
            patchelf --add-rpath ${pkgs.lib.makeLibraryPath (runtimeLibraries pkgs)} \
              $out/bin/mrtklib-egui-${app}
            addDriverRunpath $out/bin/mrtklib-egui-${app}
          '') apps;
          meta.license = with pkgs.lib.licenses; [
            bsd2
            mit
            asl20
          ];
        };
      });

      apps = forSystems (
        pkgs:
        nixpkgs.lib.genAttrs apps (app: {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/mrtklib-egui-${app}";
        })
      );

      devShells = forSystems (pkgs: {
        default = pkgs.mkShell {
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.rustfmt
            pkgs.clippy
            pkgs.rumdl
          ];
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (runtimeLibraries pkgs) + ":/run/opengl-driver/lib";
        };
      });
    };
}
