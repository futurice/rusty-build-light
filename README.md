# Rusty Build Light
It's a build light for Raspberry Pi. In Rust! Designed for the Finavia project, and set up to talk to its Jenkins, Team City and Unity Cloud CI pipelines.

* [Requirements](#requirements)
* [Building locally](#building-locally)
* [Building for ARM](#building-for-arm)
* [Setting up the Raspberry Pi](#setting-up-the-raspberry-pi)
* [Running and Configuring the Build Light](#running-and-configuring-the-build-light)
* [Circuit Diagram](#circuit-diagram)

## Requirements

 * OpenSSL and development headers (for local development).
 * pkg-config (for local development).
 * Some version of Linux. Sorry. WSL works though!
 * Rust. Mostly easily acquired via [rustup](https://www.rustup.rs/).
 * Cross-compilation: Docker. 
 * If cross-compiling without Docker, you'll need a version of OpenSSL compiled for ARM. Which you'll probably have to do yourself.

 ## Building locally

 (i.e. not-ARM)

If you don't have them already, you'll need OpenSSL development headers. On Ubuntu and its derivatives, they can be acquired by: `sudo apt-get install libssl-dev`.
You'll also need pkg-config: `sudo apt-get install pkg-config`.

When compiling locally, you should enable the WiringPi crate's feature flag, which stubs out calls to the GPIO pins, and replaces them with print-to-console.

Then, it should just be

```bash
$ cargo build --features wiringpi/development
```

If you have set up you environment for cross-compilation (see below), it would be:

```bash
$ cargo build --target=arm-unknown-linux-gnueabihf
```

## Building for ARM

### Cross-compilation from x86 to ARM

The easiest path is to use Docker.

#### With Docker
Build the docker image from the `Dockerfile`

```bash
$ cd docker
$ docker build . --tag "rust-arm-openssl"
$ docker run --rm --name "rust-arm-openssl" -v /absolute/path/to/source/folder:/source rust-arm-openssl
```

Now, run the created container:

```bash
$ docker start -a rust-arm-openssl
```

#### Without Docker

Yikes. You sure?

Okay.

First off, don't even try this on Windows. It's not worth the pain. WSL aka Bash On Windows works fine though.

**Step one** is acquire a version of OpenSSL comaptible with v0.9.23 (or whatever we have in Cargo.lock) of the `openssl` crate. As of this writing, [that's 1.1.0g](https://github.com/openssl/openssl/archive/OpenSSL_1_1_0g.tar.gz).

---

**Step two** is to compile OpenSSL for an ARM architecture. You'll need to get some things for that. The package names that follow are for Ubuntu. Substitue as necessary for your distro:
 * `build-essential` for gcc
 * `gcc-arm-linux-gnueabihf` the ARM target for gcc
 * `libssl-dev` OpenSSL dev headers
 * (Optional) `pkg-config` this is only necessary if you'd like to compile the project in x86 form (which is handy for seeing if it will compile more quickly!)

---

 **Step three** Set up the OpenSSL build environment. Untar/unzip the sources you got back in step one somewhere. Let's say into `~/src/openssl-src`, for convenience sake. Make another folder for the output. Something like `~/src/arm-openssl-output`. Yeah, that sounds good.

---

 **Step four** Let's get cracking.

 ```bash
 $ cd ~/src/openssl-src
 $ ./Configure linux-generic32 shared --prefix=/home/<your-username>/src/arm-openssl-output --openssldir=~/src/arm-openssl-output/openssl --cross-compile-prefix=/usr/bin/arm-linux-gnueabihf-      
 ```

Explanation:

 * `linux-generic32`: Compiles for a 32-bit system, as the RPi is one.
 * `shared`: Creates a static (.a) and shared (.so) library.
 * `--prefix=<absolute path to precompiled-openssl-arm folder>`: This is where the ultimate, compiled lib will be placed once compilation completes.
 * `--openssldir=<precompiled-openssl-arm folder>/openssl`: Contains config files for the built library.
 * `--cross-compile-prefix=/usr/bin/arm-linux-gnueabihf-`: NOTE THE TRAILING DASH. This tells OpenSSL where to find the cross-compiler. By default, this is where `gcc-arm-linux-gnueabihf` gets installed to, but if you put it somewhere else, give it that path instead.

 Now we're all set. Hit it.

 ```bash
 $ make depend
 $ make
 $ make install
 ```
 
 `Make install` is what actually spits everything out into `~/src/arm-openssl-output`.

 And add an environment variable that points to that output, the `openssl` crate will use it later:

 ```bash
 $ export ARM_UNKNOWN_LINUX_GNUEABIHF_OPENSSL_DIR=~/src/arm-openssl-output
 ```

---

 **Step five** Install rust via rustup:

 ```bash
 $ curl https://sh.rustup.rs -sSf | sh
 ```

 ...and add the `arm-unknown-linux-gnueabihf` target:

 ```bash
 $ rustup target add arm-unknown-linux-gnueabihf
 ```

 ---

 **Step six** Configure cargo

This repository includes it by default, but I'm noting it here for posterity:

 * In the project root, create a folder called `.cargo`.
 * In this folder, create file named `config`. Place the following inside config:

 ```toml
 [target.arm-unknown-linux-gnueabihf]
linker = "arm-linux-gnueabihf-gcc"
 ```

 * This tells Cargo to use the ARM linker if it needs to use a linker for C code. 

 **Step seven** Actually Building The Project

 Now, we should finally be able to build:

 ```bash
 cargo build --target=arm-unknown-linux-gnueabihf
 ```

 ## Setting up the Raspberry Pi

Three things are required: an OpenVPN profile with access to the CI servers, auto-reconnect to Wi-Fi on boot, and Pi needs to be set up to autostart the program on boot.

### OpenVPN
 
 * Visit https://confluence.futurice.com/display/usup/How+to+use+VPN to learn how to obtain a Futurice VPN certificate. 
 * Log into the VPN service as `finavia-vpn`. The password for this user can be found in the [password safe](https://password.futurice.com/).
 * Transfer the `.ovpn` file to the Raspberry Pi, and place it in `/etc/openvpn`.
 * Rename the `.ovpn` file to a `.conf`. 
 * Open the new `.conf` file, and add the line `auth-user-pass auth.txt` near the top.
 * Create a file named `auth.txt` in `etc/openvpn`. Its contents should just be a.) Your VPN username on the first line and, b.) Your VPN password on the second.
 * Go to `/etc/default`, and open the `openvpn` file. Uncomment (or add, if missing) a line that says `AUTOSTART="all"`.

 Done!

### Wi-Fi

* Something something dhcpcd. The old info here was wrong, need to check how the airport Pi is set up.
* Now, we need to create `wpa_supplicant.conf`. Create it at `/etc/wpa_supplicant/wpa_supplicant.conf`. 
* Your `wpa_supplicant.conf` entry will _most likely_ need to look something like the following for your network:
```bash
network={
    ssid="YOUR_NETWORK_NAME"
    psk="YOUR_NETWORK_PASSWORD"
    key_mgmt=WPA-PSK
}
```

Done!

### Autostarting the Program on Pi Boot

If we're using Raspbian "Jessie" or later, we can use systemd. If you're using something earlier...Google it, I dunno.

To create a new systemd service, create a file named `build-light.service` in `/lib/systemd/system`. Its contents will need to look something like the following:

```ini
[Unit]
Description=The Finavia Project build light.

[Service]
ExecStart=/bin/bash -c /absolute/path/to/the/rusty/build/light/executable
WorkingDirectory=/absolute/path/to/directory/containing/rusty/build/light/and/its/config/files
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
```

Then, register and start the service:

```bash
$ sudo systemctl enable build-light.service
$ sudo systemctl start build-light.service
```

The build light can now be entirely controlled by `systemctl`, and its system-level logs can be viewed with `journalctl -u build-light.service`.

### Autostarting Chromium to go to Azure on boot

Go to `/home/pi/.config/lxsession/LXDE-pi`, and add the following line:

```ini
@/usr/bin/chromium-browser --disable-restore-session-state https://portal.azure.com
```

## Running and Configuring the Build Light

The application can be configured through the use of two configuration files: 

* `log4rs.yml` for loggging. Changes to this file will be auto-detected every thirty seconds.
* and `config.toml` for application settings. Changes to this file are only read on application startup.

Both of these files are necessary, and must be in the same directory as the `rusty_build_light` executable. They are copied from `/config` to the output directory as part of the build process (see `build.rs`).

The repository includes an example `config.toml` which is mostly blank, and commented to assist with usage.

Once the files are in place, running the application is as simple as:
```bash
$ /.rusty_build_light
```

## Circuit diagram

TBD. Text for now:

The build light assumes 3 RGB LEDs, one with each color LED being driven by a Raspberry Pi GPIO pin. The pins used are configurable in `config.toml` and given in `RGB` order.
