#!/usr/local/bin/perl

use File::Compare;

system("\rm tv.out");
system("decg722 tv.g192 tv.out");

if (compare("tv.out","tv.ref")==0){
print "\nPassed simple bit-exact test\n\n";
}
else{
print "\nFailed simple bit-exact test\n\n";
}
