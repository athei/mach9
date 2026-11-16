#!/usr/bin/perl
# check-save-slots.pl <llvm-objdump> <pe>: exit 1 and name each function of
# <pe> that stores into a stack slot its own prologue saved a callee-saved
# register to, before the epilogue restores that register. llvm-mingw does
# that in some preserve_all functions (llvm/llvm-project#242966), and the
# epilogue then hands the caller a local's value in that register.
#
# It exits 2 instead of passing when it cannot have looked at the code: when
# llvm-objdump fails, when <pe> has fewer than 100 function symbols (it needs
# the COFF symbol table, which names the functions), or when fewer than 100
# functions, or fewer than a quarter of them, have a save of x19-x30 that it
# recognised (about 60 % do in FEX's DLLs), so a changed listing format does
# not make it pass.
#
# Within one function, in address order:
# - A save is the first store of a callee-saved register to [sp, #imm] (str,
#   stp, also with pre- or post-index writeback) before any instruction
#   writes that register. Its slot is the bytes the store wrote. Callee-saved
#   are x19-x30 and v8-v15, and in a function that saves one of x9-x15 that
#   way anywhere, as preserve_all functions do, also x9-x15 and v16-v31.
# - sp is followed through sub/add sp, sp, #imm and pre- and post-index
#   writeback of loads and stores. Any other write to sp (a realignment,
#   __chkstk's sub sp, sp, xN) ends the check of that function, as slot
#   addresses are unknown after it.
# - A load of the register from sp ends its slot until the next ret, b or br:
#   an epilogue may reuse the freed slot. The code after a ret, b or br runs
#   on other paths with the frame set up, at the sp from before the epilogue
#   that ends there, so the slots count again there. That epilogue is the run
#   of instructions before the ret, b or br back to the last store, call,
#   branch or sub sp.
# - A finding is a store of another register, of any width, to sp-relative
#   bytes of a counted slot. Not looked at, so they can only hide a finding:
#   stores through x29 or a register offset, and the stores the patterns
#   above do not name (st1 to st4, stlr, stlur, stxr, stlxr, stg and the
#   other atomic and tagged stores).
# - Two layouts could make it fail a correct build, by name, for the
#   maintainer to judge: a body-level add sp, sp, #imm with no store, call or
#   branch between it and an epilogue is taken as part of that epilogue, so
#   the code after its ret is checked at the wrong sp; and a shrink-wrapped
#   path laid out after an epilogue that runs without the frame is checked
#   against slots that do not exist on that path. FEX's DLLs give no such
#   false finding today.
use strict;
use warnings;

my ($objdump, $pe) = @ARGV;
die "usage: $0 <llvm-objdump> <pe>\n" unless defined $pe;
open(my $in, '-|', $objdump, '-d', '--no-show-raw-insn', $pe) or die "cannot run $objdump: $!\n";

my %width = (x => 8, w => 4, q => 16, d => 8, s => 4, h => 2, b => 1);
my ($functions, $instructions, $with_saves) = (0, 0, 0);
my (@found, $maybe_pa);

sub reg_id {
    my ($r) = @_;
    return "x$1" if $r =~ /^[xw](\d+)$/;
    return "v$1" if $r =~ /^[qdshbv](\d+)$/;
    return undef;
}

sub num {
    my ($s) = @_;
    return 0 unless defined $s;
    my $neg = $s =~ s/^-//;
    my $v = $s =~ /^0x/ ? hex($s) : $s;
    return $neg ? -$v : $v;
}

# One instruction: [addr, op, args, regs of a [sp] operand, imm, writeback,
# post-index imm, is a [sp] access].
sub parse {
    my ($line) = @_;
    return undef unless $line =~ /^\s*([0-9a-f]+):\s+(\S+)\s*(.*?)\s*$/;
    my ($addr, $op, $args) = ($1, $2, $3);
    $args =~ s/\s*(\/\/|<).*$//;
    return [$addr, $op, $args] if index($args, '[sp') < 0;
    my @m = $args =~ /^(.*?),?\s*\[sp(?:, #(-?0x[0-9a-f]+|-?\d+))?\](!)?(?:, #(-?0x[0-9a-f]+|-?\d+))?$/;
    return [$addr, $op, $args, @m ? ($m[0], num($m[1]), $m[2], num($m[3]), 1) : ('', 0, undef, 0, 0)];
}

sub is_store { return $_[0] =~ /^(str|stp|strb|strh|stur|sturb|sturh|stnp)$/ }

sub dests {
    my ($op, $args) = @_;
    return ('x30') if $op eq 'bl' || $op eq 'blr';
    return () if $op =~ /^(st|cmp|cmn|tst|fcmp|ccmp|b\.|cb|tb|prfm|hint|nop|dmb|dsb|isb|msr|brk|udf|svc|ret|b$|br$)/;
    my ($first, $second) = $args =~ /^([xwqdshbv]\d+)(?:\.\w+)?(?:, ([xwqdshbv]\d+))?/ or return ();
    return grep { defined } (reg_id($first), $op =~ /^ldp/ && defined $second ? reg_id($second) : undef);
}

sub check {
    my ($name, @ins) = @_;
    return unless @ins;
    $instructions += @ins;

    # The preserve_all shape: a save of one of x9-x15 anywhere. Only looked
    # for when the function stores one of them to the stack at all.
    my ($pa, %w) = (0);
    for my $i ($maybe_pa ? @ins : ()) {
        my ($addr, $op, $args, $regs, $n, $wb, $p, $base) = @$i;
        if ($base && is_store($op)) {
            for my $id (grep { defined } map { reg_id($_) } split /\s*,\s*/, $regs) {
                $pa = 1 if !$w{$id} && $id =~ /^x(\d+)$/ && $1 >= 9 && $1 <= 15;
            }
        }
        $w{$_} = 1 for dests($op, $args);
    }
    my $callee_saved = sub {
        my ($id) = @_;
        my ($kind, $n) = $id =~ /^([xv])(\d+)$/ or return 0;
        return ($n >= 19 && $n <= 30) || ($pa && $n >= 9 && $n <= 15) if $kind eq 'x';
        return ($n >= 8 && $n <= 15) || ($pa && $n >= 16);
    };

    my ($sp, %saved, %slot, %written, @sp_at) = (0);
    my $gpr_save = 0;
    for my $k (0 .. $#ins) {
        my ($addr, $op, $args, $regs, $n, $wb, $p, $base) = @{$ins[$k]};
        $sp_at[$k] = $sp;
        if ($op =~ /^(ret|b|br)$/) {
            # Back over the epilogue that ends here to the sp before it.
            my $body = $sp;
            for (my $j = $k - 1; $j >= 0; $j--) {
                my ($jo, $ja, $jbase, $jp) = @{$ins[$j]}[1, 2, 7, 6];
                last if is_store($jo) || $jo =~ /^(bl|blr|b|br|ret|b\..*|cbn?z|tbn?z)$/;
                last if $jo eq 'sub' && $ja =~ /^sp, sp, #/;
                $body = $sp_at[$j] if ($jo eq 'add' && $ja =~ /^sp, sp, #/) || ($jbase && $jp && $jo =~ /^ld/);
            }
            $sp = $body;
            $slot{$_} = $saved{$_} for keys %saved;
            next;
        }
        if ($op =~ /^(sub|add)$/ && $args =~ /^sp, sp, #(0x[0-9a-f]+|\d+)(, lsl #12)?$/) {
            my ($v, $shift) = ($1, $2);
            $v = num($v) << ($shift ? 12 : 0);
            $sp += $op eq 'add' ? $v : -$v;
            next;
        }
        return if $args =~ /^sp\b/ && $op !~ /^(cmp|cmn|tst)$/;
        my @r = $base ? grep { defined } map { reg_id($_) } split /\s*,\s*/, $regs : ();
        if ($base && is_store($op)) {
            my $each = $op =~ /b$/ ? 1 : $op =~ /h$/ ? 2 : ($width{substr($regs, 0, 1)} // 8);
            my $size = $op =~ /^(stp|stnp)$/ ? $each * @r : $each;
            $sp += $n if $wb;
            my $start = $sp + ($wb ? 0 : $n);
            my %own = map { $_ => 1 } @r;
            for my $id (sort keys %slot) {
                my ($s, $sw) = @{$slot{$id}};
                next if $own{$id} || $start >= $s + $sw || $s >= $start + $size;
                push @found, sprintf('%s: %s saved at entry sp%s%#x, overwritten at %s by %s %s',
                                     $name, $id, $s < 0 ? '-' : '+', abs($s), $addr, $op, $args);
                delete $slot{$id};
            }
            for my $i (0 .. $#r) {
                my $id = $r[$i];
                next if exists $saved{$id} || $written{$id} || !$callee_saved->($id);
                $saved{$id} = $slot{$id} = [$start + $i * $each, $each];
                $gpr_save = 1 if $id =~ /^x(\d+)$/ && $1 >= 19;
            }
            $sp += $p;
            next;
        }
        if ($base && $op =~ /^ld/) {
            $sp += $n if $wb;
            delete $slot{$_} for @r;
            $sp += $p;
        }
        $written{$_} = 1 for dests($op, $args);
    }
    $with_saves++ if $gpr_save;
}

my ($name, @ins);
while (my $line = <$in>) {
    if ($line =~ /^[0-9a-f]+ <(.+)>:$/) {
        my $next = $1;
        check($name, @ins) if defined $name;
        ($name, $maybe_pa, @ins) = ($next, 0);
        $functions++;
        next;
    }
    next unless defined $name;
    my $i = parse($line);
    next unless $i;
    push @ins, $i;
    $maybe_pa = 1 if $i->[1] =~ /^st/ && $i->[2] =~ /^(?:[xw]\d+, )?[xw](?:9|1[0-5])\b.*\[sp/;
}
check($name, @ins) if defined $name;
if (!close($in)) {
    print "Error: $objdump -d $pe failed\n";
    exit 2;
}

if ($functions < 100) {
    print "Error: $pe has $functions function symbols; the save slot check needs its COFF symbol table\n";
    exit 2;
}
if ($with_saves < 100 || $with_saves * 4 < $functions) {
    print "Error: in $pe, only $with_saves of $functions functions ($instructions instructions read) have a save of x19-x30 this check recognised; the llvm-objdump listing does not look as it expects\n";
    exit 2;
}
if (@found) {
    print "Error: $pe has functions that overwrite the saved value of a callee-saved register:\n";
    print "  $_\n" for @found;
    print "This is llvm/llvm-project#242966; see FEX in docs/maintenance.md.\n";
    exit 1;
}
print "    $pe: no function overwrites a register save slot ($functions functions, $with_saves with saves)\n";
