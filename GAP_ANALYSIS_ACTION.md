Below are the user requested actions for each  gap analysis item.
From it, make an implementation plan for each of them with the gap description, the context of it and what to do. This plan will be given to a fresh agent.
If an action require a change to the C++ repo, report in a separate file so the C++ repo can be improved later on.

Any difference that are accepted as is should be reported in another file taht will be committed so a future agent can read and know that it is an accepted gap.

F-01: Add the implementation. Including global comm test like the others.
F-02: Add the missing check, mirror the C++ side
F-03: Ignore, bool16,32,64 don't exist in rust. Make sure they are rejected if a server tries toa ccess them.
F-04: Leave like this
F-05: Leave like this
F-06: A loop handler should eb able to exist without datalogging enabled. But the invokation will not support manipulating the datalogger.
F-07: Add the missing function
F-08: Do the clear
F-09: Make sure this can extended in the future. Leaving AtomicFlag for now is fine
F-10: Do nothing
F-11: Do nothing
F-12: Do nothing

D-01: Make it enabled by default. reverse the logic, need an explicit disable
D-02: Leave like this. Report as C++ improvement
D-03: Required for ownership transfer.
D-04: Ignore
D-05:  Ignore
D-06: Replicate C++ behavior and test
D-07: Report as C++ improvement
D-08: Report as c++ improvement
D-09: Report as C++ improvement )might be already implemented in a branch)
D-10: Report as c++ imrpovement
D-11: On purpose

A-01: On purpose. Leave as is
A-02: On purpose. Leave as is
A-03: Fine, comes with Rust idioms
A-04: Make them match
A-05: Leave like this
A-06: I believe it can't as the datalogger ownership is passed across time domain without asking the compiler permission, making it unsafe operation. If we can change the API without rearchitecturing the ownership model, fix it. Otherwise, leave as is.
A-07: Do nothing

Q-01: Remove and replace by a silent cancel (go to error state)
Q-02 : On purpose.

T-01: Add mising tests
T-02: Add mising tests
T-03: Add mising tests
T-04: Add mising tests
T-05: Add mising tests
T-06: Add mising tests

T-07: add missing test
T-08: add missing test
T-09: add missing test
T-10: add missing test
T-11: add missing test
T-12: Ignore.