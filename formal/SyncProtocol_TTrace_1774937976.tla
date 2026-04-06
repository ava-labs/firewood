---- MODULE SyncProtocol_TTrace_1774937976 ----
EXTENDS Sequences, TLCExt, Toolbox, SyncProtocol, Naturals, TLC

_expression ==
    LET SyncProtocol_TEExpression == INSTANCE SyncProtocol_TEExpression
    IN SyncProtocol_TEExpression!expression
----

_trace ==
    LET SyncProtocol_TETrace == INSTANCE SyncProtocol_TETrace
    IN SyncProtocol_TETrace!trace
----

_inv ==
    ~(
        TLCGet("level") = Len(_TETrace)
        /\
        verifierTrie = ({})
        /\
        startKey = (None)
        /\
        synced = (TRUE)
        /\
        round = (1)
        /\
        endRootTrie = ({})
        /\
        startRootTrie = ({})
    )
----

_init ==
    /\ synced = _TETrace[1].synced
    /\ endRootTrie = _TETrace[1].endRootTrie
    /\ verifierTrie = _TETrace[1].verifierTrie
    /\ round = _TETrace[1].round
    /\ startKey = _TETrace[1].startKey
    /\ startRootTrie = _TETrace[1].startRootTrie
----

_next ==
    /\ \E i,j \in DOMAIN _TETrace:
        /\ \/ /\ j = i + 1
              /\ i = TLCGet("level")
        /\ synced  = _TETrace[i].synced
        /\ synced' = _TETrace[j].synced
        /\ endRootTrie  = _TETrace[i].endRootTrie
        /\ endRootTrie' = _TETrace[j].endRootTrie
        /\ verifierTrie  = _TETrace[i].verifierTrie
        /\ verifierTrie' = _TETrace[j].verifierTrie
        /\ round  = _TETrace[i].round
        /\ round' = _TETrace[j].round
        /\ startKey  = _TETrace[i].startKey
        /\ startKey' = _TETrace[j].startKey
        /\ startRootTrie  = _TETrace[i].startRootTrie
        /\ startRootTrie' = _TETrace[j].startRootTrie

\* Uncomment the ASSUME below to write the states of the error trace
\* to the given file in Json format. Note that you can pass any tuple
\* to `JsonSerialize`. For example, a sub-sequence of _TETrace.
    \* ASSUME
    \*     LET J == INSTANCE Json
    \*         IN J!JsonSerialize("SyncProtocol_TTrace_1774937976.json", _TETrace)

=============================================================================

 Note that you can extract this module `SyncProtocol_TEExpression`
  to a dedicated file to reuse `expression` (the module in the 
  dedicated `SyncProtocol_TEExpression.tla` file takes precedence 
  over the module `SyncProtocol_TEExpression` below).

---- MODULE SyncProtocol_TEExpression ----
EXTENDS Sequences, TLCExt, Toolbox, SyncProtocol, Naturals, TLC

expression == 
    [
        \* To hide variables of the `SyncProtocol` spec from the error trace,
        \* remove the variables below.  The trace will be written in the order
        \* of the fields of this record.
        synced |-> synced
        ,endRootTrie |-> endRootTrie
        ,verifierTrie |-> verifierTrie
        ,round |-> round
        ,startKey |-> startKey
        ,startRootTrie |-> startRootTrie
        
        \* Put additional constant-, state-, and action-level expressions here:
        \* ,_stateNumber |-> _TEPosition
        \* ,_syncedUnchanged |-> synced = synced'
        
        \* Format the `synced` variable as Json value.
        \* ,_syncedJson |->
        \*     LET J == INSTANCE Json
        \*     IN J!ToJson(synced)
        
        \* Lastly, you may build expressions over arbitrary sets of states by
        \* leveraging the _TETrace operator.  For example, this is how to
        \* count the number of times a spec variable changed up to the current
        \* state in the trace.
        \* ,_syncedModCount |->
        \*     LET F[s \in DOMAIN _TETrace] ==
        \*         IF s = 1 THEN 0
        \*         ELSE IF _TETrace[s].synced # _TETrace[s-1].synced
        \*             THEN 1 + F[s-1] ELSE F[s-1]
        \*     IN F[_TEPosition - 1]
    ]

=============================================================================



Parsing and semantic processing can take forever if the trace below is long.
 In this case, it is advised to uncomment the module below to deserialize the
 trace from a generated binary file.

\*
\*---- MODULE SyncProtocol_TETrace ----
\*EXTENDS IOUtils, SyncProtocol, TLC
\*
\*trace == IODeserialize("SyncProtocol_TTrace_1774937976.bin", TRUE)
\*
\*=============================================================================
\*

---- MODULE SyncProtocol_TETrace ----
EXTENDS SyncProtocol, TLC

trace == 
    <<
    ([verifierTrie |-> {},startKey |-> None,synced |-> FALSE,round |-> 0,endRootTrie |-> {},startRootTrie |-> {}]),
    ([verifierTrie |-> {},startKey |-> None,synced |-> TRUE,round |-> 1,endRootTrie |-> {},startRootTrie |-> {}])
    >>
----


=============================================================================

---- CONFIG SyncProtocol_TTrace_1774937976 ----
CONSTANTS
    BF = 2
    MaxDepth = 2
    None = None
    None = None

INVARIANT
    _inv

CHECK_DEADLOCK
    \* CHECK_DEADLOCK off because of PROPERTY or INVARIANT above.
    FALSE

INIT
    _init

NEXT
    _next

CONSTANT
    _TETrace <- _trace

ALIAS
    _expression
=============================================================================
\* Generated on Tue Mar 31 02:19:38 EDT 2026