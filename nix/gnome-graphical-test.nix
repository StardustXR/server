{ pkgs, lib ? pkgs.lib, self, ...  }:
# Some code is copy-pasted from https://github.com/NixOS/nixpkgs/blob/master/nixos/tests/gnome.nix
# TODO: make this less boiler-platey and make a function like mkGnomeTest that does all this and upstream it to nixpkgs
{
  name = "stardust-xr-server-gnome-vmtest";
  meta = with lib; {
    maintainers = [ maintainers.matthewcroughan ];
  };
  nodes.machine = { ... }: {
    imports = [ "${pkgs.path}/nixos/tests/common/user-account.nix" ];
    virtualisation.qemu.options = [
      "-device virtio-gpu-pci"
    ];
    environment.systemPackages = [ pkgs.monado ];
    services.xserver = {
      enable = true;
      desktopManager.gnome = {
        enable = true;
        debug = true;
        # Set a nice desktop background that is pleasing to the eyes :3
        extraGSettingsOverrides = ''
          [org.gnome.desktop.background]
          picture-uri='file://${pkgs.gnome-backgrounds}/share/backgrounds/gnome/blobs-l.svg'
          picture-uri-dark='file://${pkgs.gnome-backgrounds}/share/backgrounds/gnome/blobs-l.svg'
        '';
      };
      displayManager = {
        gdm = {
          enable = true;
          debug = true;
        };
        autoLogin = {
          enable = true;
          user = "alice";
        };
      };
    };
    systemd.user.services = {
      "monado" = {
        after = [ "graphical-session.target" "default.target" "org.gnome.Shell@wayland.service" ];
        environment = {
          XRT_COMPOSITOR_FORCE_WAYLAND = "1";
          WAYLAND_DISPLAY = "wayland-0";
        };
        serviceConfig = {
          ExecStartPre = [
            "${pkgs.writeShellScript "sleep" ''
              sleep 3
            ''}"
          ];
          ExecStart = let
            # stdin disappears in NixOS test driver ( machine.succeed() ), requiring us to specify < /dev/ttyS0 to fake stdin
            exec-monado-service = pkgs.writeShellScript "exec-monado-service" "${pkgs.monado}/bin/monado-service < /dev/ttyS0";
          in [
            "${exec-monado-service}"
          ];
        };
      };
      "stardust-xr-server" = {
        after = [ "monado.service" ];
        serviceConfig = {
          Type = "notify";
          NotifyAccess = "all";
          ExecStartPre = [
            "${pkgs.writeShellScript "sleep" ''
              sleep 3
            ''}"
          ];
          ExecStart = let
            notifyReady = pkgs.writeShellScript "notifyReady" "systemd-notify --ready";
            exec-stardust-xr-server = pkgs.writeShellScript "exec-stardust-xr-server" "${self.packages.${pkgs.hostPlatform.system}.default}/bin/stardust-xr-server -e ${notifyReady}";
          in [
            "${exec-stardust-xr-server}"
          ];
        };
      };
      "weston-cliptest" = {
        after = [ "flatland.service" ];
        environment.WAYLAND_DISPLAY = "wayland-1";
        serviceConfig = {
          ExecStart = [
            "${pkgs.weston}/bin/weston-cliptest"
          ];
        };
      };
      "flatland" = {
        after = [ "stardust-xr-server.service" ];
        serviceConfig = {
          ExecStart = [
            "${self.inputs.flatland.packages.${pkgs.hostPlatform.system}.default}/bin/flatland"
          ];
        };
      };
      "org.gnome.Shell@wayland" = {
        wants = [ "monado.service" "stardust-xr-server.service" "flatland.service" "weston-cliptest.service" ];
        serviceConfig = {
          ExecStart = [
            # Clear the list before overriding it.
            ""
            # Eval API is now internal so Shell needs to run in unsafe mode.
            # TODO: improve test driver so that it supports openqa-like manipulation
            # that would allow us to drop this mess.
            "${pkgs.gnome-shell}/bin/gnome-shell --unsafe-mode"
          ];
        };
      };
    };
  };

  testScript = { nodes, ... }: let
    # Keep line widths somewhat managable
    user = nodes.machine.config.users.users.alice;
    uid = toString user.uid;
    bus = "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/${uid}/bus";
    gdbus = "${bus} gdbus";
    su = command: "su ${user.name} -c '${command}'";

    # Call javascript in gnome shell, returns a tuple (success, output), where
    # `success` is true if the dbus call was successful and output is what the
    # javascript evaluates to.
    eval = "call --session -d org.gnome.Shell -o /org/gnome/Shell -m org.gnome.Shell.Eval";

    # False when startup is done
    startingUp = su "${gdbus} ${eval} Main.layoutManager._startingUp";
  in ''
      with subtest("Login to GNOME with GDM"):
          # wait for gdm to start
          machine.wait_for_unit("display-manager.service")
          # wait for the wayland server
          machine.wait_for_file("/run/user/${uid}/wayland-0")
          # wait for alice to be logged in
          machine.wait_for_unit("default.target", "${user.name}")
          # check that logging in has given the user ownership of devices
          assert "alice" in machine.succeed("getfacl -p /dev/snd/timer")

      with subtest("Wait for GNOME Shell"):
          # correct output should be (true, 'false')
          machine.wait_until_succeeds(
              "${startingUp} | grep -q 'true,..false'"
          )

      # To allow monado-service to use < /dev/ttyS0
      machine.succeed("chown alice /dev/ttyS0")

      with subtest("Open Monado and StardustXR"):
          # Close the Activities view so that Shell can correctly track the focused window.
          machine.send_key("esc")
          machine.wait_for_unit("monado.service", "${user.name}")
          machine.wait_for_unit("stardust-xr-server.service", "${user.name}")
          machine.wait_for_unit("flatland.service", "${user.name}")
          machine.wait_for_unit("weston-cliptest.service", "${user.name}")
          machine.sleep(3)
          machine.screenshot("screen")
    '';
}
