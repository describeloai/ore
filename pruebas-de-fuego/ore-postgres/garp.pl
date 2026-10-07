# garp.pl <iface> <ip> — un ARP gratuito («<ip> soy yo, en mi MAC») por un socket crudo (P3·6). Lo usa
# p36.sh dentro del huésped (que tiene perl, no python ni un arping que deje elegir). Sólo pasa si <ip>
# es la de la VM: el filtro de su runner tira cualquier otra (malla/postgres-computo/neonvm/cerrada.yaml).
use Socket;
my ($if,$ip)=@ARGV;
open F,"/sys/class/net/$if/address"; my $m=<F>; chomp $m; $m=~s/://g; my $mac=pack("H12",$m);
open G,"/sys/class/net/$if/ifindex"; my $idx=<G>; chomp $idx;
socket(S,17,3,0) or die "socket $!";
my $sll=pack("S n i S C C a8",17,0x0806,$idx,1,0,6,"\xff" x 6);
my $ipb=inet_aton($ip);
my $arp=pack("n n C C n",1,0x0800,6,4,1).$mac.$ipb.("\0" x 6).$ipb;
my $fr=("\xff" x 6).$mac.pack("n",0x0806).$arp;
for (1..3){ send(S,$fr,0,$sll) or die "send $!"; }
print "enviado: $ip es $m\n";
