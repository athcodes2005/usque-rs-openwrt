## How did I set it up using OpenWRT for IPv6 routing?

I wanted to set up WARP as the default IPv6 exit for my home internet since my ISP doesn't provide one. And I keep IPv4 untouched. This docs below explains the steps I did to get it up and running.

**Note**: This is on a vanilla OpenWRT installation, your mileage may vary.

**I DO NOT TAKE ANY RESPONSIBILITY FOR BRICKED ROUTERS, BROKEN DEVICES AND WHATSOEVER**. Use common sense.

0. `opkg update && install kmod-tun` - this is important, make sure that `lsmod | grep tun` shows the loaded kernel module after!
1. Created the work dir: `mkdir /root/usque`
2. Moved the extracted binary: `cat ./usque-rs | ssh root@192.168.1.1 "cat > /root/usque/usque-rs && chmod +x /root/usque/usque-rs"`
3. Ran the login: `cd /root/usque && ./usque-rs login`, accepted EULA
4. Ran `vi /etc/init.d/usque-rs`:
```sh
#!/bin/sh /etc/rc.common

USE_PROCD=1
START=95

APP_DIR="/root/usque"

start_service() {
    procd_open_instance
    
    procd_set_param command /bin/sh -c "cd $APP_DIR && exec ./usque-rs nativetun"

    procd_set_param stdout 1
    procd_set_param stderr 1
    procd_set_param respawn
    procd_close_instance
}
```
5. `chmod +x /etc/init.d/usque-rs`
6. `/etc/init.d/usque-rs enable && /etc/init.d/usque-rs start`
7. Check its status using: ` /etc/init.d/usque-rs status`. If not running, check logs with `logread -e usque` and fix your issues.
8. `ip a show dev tun0` should display the interface.
9. Go to Network → Interfaces, click _Add new interface_, name: WARP, protocol: Unmanaged; device: tun0 (If it doesn't appear in the dropdown, just type `tun0` manually).
10. Now that the interface is known by OpenWRT, find it under Network → Interfaces, click Edit, then in Advanced Settings untick _Use Advanced Gateway_ and in Firewall Settings assign the newly created `WARP` interface to the `wan` zone.
11. In Network → Firewall scroll down to zones, find the one that starts with `wan`, `REJECT`, `reject`, `accept`, `reject` (this will be your `wan` zone) and click Edit. Go to its Advanced Settings and tick _IPv6 Masquerading_. We only get a /128 from Cloudflare (unfortunately), so we have to NAT.
12. Find your lan interface under Network → Interfaces. Click Edit. Then pick the DHCP Server tab. Under that pick the IPv6 Settings subtab and make sure that RA-Service is set to `server mode`, DHCPv6 service is also set to `server mode` and NDP-Proxy is `disabled`. These were the default values on my end, but making sure. Then switch to the IPv6 RA Settings subtab and switch the Default router to forced. Without that stuff didn't work for me.
13. Go to Network → Routing. Pick Static IPv6 Routes. Add a new one. For the interface pick `WARP`. Set target to `::/0` to route anything or fine grain it however you want. In Advanced Settings I set the Metric to 10 but you can skip that.

And voila, IPv6 started working on all my IPv6 enabled clients that understood RAs. :)